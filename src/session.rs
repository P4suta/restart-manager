//! Role-aware session lifecycle and owned recovery typestates.

use std::borrow::Borrow;
use std::ffi::OsStr;
use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, Weak};

use crate::application::{
    AffectedApplication, AffectedApplications, ApplicationStatus, ApplicationType, ProcessIdentity,
    RebootReasons,
};
use crate::error::{Error, ErrorKind, ParseSessionKeyError, Result};
use crate::filter::{Filter, FilterAction, FilterTarget};
use crate::resource::ResourceBatch;
use crate::shutdown::{OperationOutcome, Progress, RecoveryOutcome, ShutdownOptions};
use crate::sys::{
    self, RawAffectedApplications, RawFilter, RawFilterTarget, RawUniqueProcess, SessionHandle,
    SysError,
};

const INVALID_NATIVE_ID: u32 = u32::MAX;
const ERROR_FILE_NOT_FOUND: u32 = 2;
const ERROR_ACCESS_DENIED: u32 = 5;
const ERROR_INVALID_HANDLE: u32 = 6;
const ERROR_OUTOFMEMORY: u32 = 14;
const ERROR_WRITE_FAULT: u32 = 29;
const ERROR_SEM_TIMEOUT: u32 = 121;
const ERROR_BAD_ARGUMENTS: u32 = 160;
const ERROR_DIRECTORY: u32 = 267;
const ERROR_FAIL_NOACTION_REBOOT: u32 = 350;
const ERROR_FAIL_SHUTDOWN: u32 = 351;
const ERROR_FAIL_RESTART: u32 = 352;
const ERROR_MAX_SESSIONS_REACHED: u32 = 353;
const ERROR_REQUEST_OUT_OF_SEQUENCE: u32 = 776;
const ERROR_SESSION_CREDENTIAL_CONFLICT: u32 = 1219;
const ERROR_CANCELLED: u32 = 1223;

/// A validated, 32-character ASCII hexadecimal Restart Manager session key.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct SessionKey(String);

impl SessionKey {
    /// Returns the key for explicit cross-process transfer.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Converts the key into its owned string.
    #[must_use]
    pub fn into_string(self) -> String {
        self.0
    }

    fn generated(value: String) -> Result<Self> {
        if is_valid_key(&value) {
            Ok(Self(value))
        } else {
            Err(Error::new(
                ErrorKind::MalformedOsData,
                None,
                "Windows returned a malformed Restart Manager session key",
            ))
        }
    }
}

impl fmt::Debug for SessionKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("SessionKey")
            .field(&"<redacted>")
            .finish()
    }
}

impl FromStr for SessionKey {
    type Err = ParseSessionKeyError;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        is_valid_key(value)
            .then(|| Self(value.to_owned()))
            .ok_or(ParseSessionKeyError)
    }
}

fn is_valid_key(value: &str) -> bool {
    value.len() == 32 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

struct SessionCore {
    handle: Arc<SessionHandle>,
    key: SessionKey,
}

impl SessionCore {
    fn start() -> Result<Self> {
        let (handle, key) = SessionHandle::start().map_err(map_sys_error)?;
        Ok(Self {
            handle: Arc::new(handle),
            key: SessionKey::generated(key)?,
        })
    }

    fn join(key: &SessionKey) -> Result<Self> {
        let handle = SessionHandle::join(key.as_str()).map_err(map_join_error)?;
        Ok(Self {
            handle: Arc::new(handle),
            key: key.clone(),
        })
    }

    fn register_resources(&self, resources: &ResourceBatch) -> Result<()> {
        if resources.is_empty() {
            return Ok(());
        }
        let files = resources
            .files()
            .map(validate_and_absolute_file)
            .collect::<Result<Vec<_>>>()?;
        let processes = resources
            .processes()
            .iter()
            .copied()
            .map(raw_process)
            .collect::<Vec<_>>();
        let services = resources
            .services()
            .map(|name| {
                validate_os_value(name, "a service short name")?;
                Ok(name.to_os_string())
            })
            .collect::<Result<Vec<_>>>()?;
        self.handle
            .register_resources(&files, &processes, &services)
            .map_err(|error| map_operation_error(error, Operation::Register))
    }

    fn affected_applications(&self) -> Result<AffectedApplications> {
        self.handle
            .affected_applications()
            .map_err(|error| map_operation_error(error, Operation::Report))
            .map(application_report)
    }

    fn end(self) -> Result<()> {
        self.handle
            .end()
            .map_err(|error| map_operation_error(error, Operation::End))
    }
}

/// A primary-installer Restart Manager session.
///
/// Shutdown consumes this value and produces a [RestartPending], so restart
/// cannot be called out of sequence.
pub struct RestartSession {
    core: SessionCore,
}

impl RestartSession {
    /// Starts a new primary-installer session.
    pub fn new() -> Result<Self> {
        SessionCore::start().map(|core| Self { core })
    }

    /// Returns the key a secondary installer can pass to [JoinedSession::join].
    #[must_use]
    pub fn session_key(&self) -> &SessionKey {
        &self.core.key
    }

    /// Registers a mixed collection in one native call.
    pub fn register_resources(&mut self, resources: &ResourceBatch) -> Result<()> {
        self.core.register_resources(resources)
    }

    /// Registers file paths. Directories are rejected by Restart Manager.
    pub fn register_files<I, P>(&mut self, files: I) -> Result<()>
    where
        I: IntoIterator<Item = P>,
        P: AsRef<Path>,
    {
        let mut resources = ResourceBatch::new();
        for file in files {
            resources.add_file(file.as_ref().to_path_buf());
        }
        self.core.register_resources(&resources)
    }

    /// Registers exact process identities from any iterator.
    pub fn register_processes<I, P>(&mut self, processes: I) -> Result<()>
    where
        I: IntoIterator<Item = P>,
        P: Borrow<ProcessIdentity>,
    {
        let mut resources = ResourceBatch::new();
        for process in processes {
            resources.add_process(*process.borrow());
        }
        self.core.register_resources(&resources)
    }

    /// Registers Windows services by short name.
    pub fn register_services<I, S>(&mut self, services: I) -> Result<()>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut resources = ResourceBatch::new();
        for service in services {
            resources.add_service(service.as_ref().to_os_string());
        }
        self.core.register_resources(&resources)
    }

    /// Takes a reusable snapshot of affected applications and reboot reasons.
    pub fn affected_applications(&mut self) -> Result<AffectedApplications> {
        self.core.affected_applications()
    }

    /// Creates a weak, thread-safe cancellation capability.
    #[must_use]
    pub fn cancellation_handle(&self) -> CancellationHandle {
        CancellationHandle {
            handle: Arc::downgrade(&self.core.handle),
        }
    }

    /// Gracefully attempts shutdown and always enters the recovery-required state.
    pub fn shutdown(self) -> RestartPending {
        self.shutdown_with_options(ShutdownOptions::default())
    }

    /// Attempts shutdown and retains its outcome regardless of native success.
    pub fn shutdown_with_options(self, options: ShutdownOptions) -> RestartPending {
        let mut pending = RestartPending::new(self.core);
        let result = pending
            .core()
            .handle
            .shutdown(options.native_flags())
            .map_err(|error| map_operation_error(error, Operation::Shutdown));
        pending.shutdown = OperationOutcome::from_result(result);
        pending
    }

    /// Attempts shutdown while delivering validated, strictly increasing progress.
    ///
    /// If the process-global callback lease cannot be acquired, the returned
    /// error also returns this session and no native operation has started.
    pub fn shutdown_with_progress<F>(
        self,
        options: ShutdownOptions,
        mut callback: F,
    ) -> std::result::Result<RestartPending, OperationNotStarted<Self>>
    where
        F: FnMut(Progress) + Send,
    {
        let mut pending = RestartPending::new(self.core);
        let mut last = None;
        let mut malformed = false;
        let result = pending
            .core()
            .handle
            .shutdown_with_progress(options.native_flags(), &mut |native| {
                deliver_progress(native, &mut last, &mut malformed, &mut callback)
            });
        if let Err(error) = result {
            if operation_not_started(error) {
                let state = Self {
                    core: pending.take_core(),
                };
                return Err(OperationNotStarted::new(state, map_sys_error(error)));
            }
            pending.shutdown =
                OperationOutcome::Failed(map_operation_error(error, Operation::Shutdown));
            return Ok(pending);
        }
        pending.shutdown = if malformed {
            OperationOutcome::Failed(malformed_progress_error())
        } else {
            OperationOutcome::Succeeded
        };
        Ok(pending)
    }

    /// Adds or replaces a restart/shutdown filter.
    pub fn set_filter(&mut self, target: &FilterTarget, action: FilterAction) -> Result<()> {
        self.core
            .handle
            .add_filter(&raw_filter_target(target), raw_filter_action(action))
            .map_err(|error| map_operation_error(error, Operation::SetFilter))
    }

    /// Removes a filter from the selected target.
    pub fn remove_filter(&mut self, target: &FilterTarget) -> Result<()> {
        self.core
            .handle
            .remove_filter(&raw_filter_target(target))
            .map_err(|error| map_operation_error(error, Operation::RemoveFilter))
    }

    /// Lists filters configured by this primary installer.
    pub fn filters(&mut self) -> Result<Vec<Filter>> {
        self.core
            .handle
            .filters()
            .map_err(|error| map_operation_error(error, Operation::Filters))?
            .into_iter()
            .map(filter_from_raw)
            .collect()
    }

    /// Ends the session without attempting shutdown.
    pub fn end(self) -> Result<()> {
        self.core.end()
    }
}

/// The state entered after any shutdown attempt.
///
/// Dropping an armed value makes one best-effort restart attempt and then ends
/// the session. This covers early returns, panics, and partial shutdown.
#[must_use = "dropping this value attempts recovery; call restart or leave_stopped explicitly"]
pub struct RestartPending {
    core: Option<SessionCore>,
    shutdown: OperationOutcome,
    armed: bool,
}

impl RestartPending {
    fn new(core: SessionCore) -> Self {
        Self {
            core: Some(core),
            shutdown: OperationOutcome::Succeeded,
            armed: true,
        }
    }

    fn core(&self) -> &SessionCore {
        self.core.as_ref().expect("pending session owns its core")
    }

    fn take_core(&mut self) -> SessionCore {
        self.armed = false;
        self.core.take().expect("pending session owns its core")
    }

    /// Returns the retained shutdown result.
    #[must_use]
    pub const fn shutdown_outcome(&self) -> &OperationOutcome {
        &self.shutdown
    }

    /// Attempts restart even when shutdown failed.
    #[must_use]
    pub fn restart(mut self) -> RecoveryCompletion {
        let result = self
            .core()
            .handle
            .restart()
            .map_err(|error| map_operation_error(error, Operation::Restart));
        let outcome = RecoveryOutcome {
            shutdown: self.shutdown.clone(),
            restart: Some(OperationOutcome::from_result(result)),
        };
        RecoveryCompletion {
            core: self.take_core(),
            outcome,
        }
    }

    /// Attempts restart with validated, strictly increasing progress.
    ///
    /// The error deliberately owns the full pending state so recovery cannot
    /// be lost merely to reduce the enum size.
    #[allow(clippy::result_large_err)]
    pub fn restart_with_progress<F>(
        mut self,
        mut callback: F,
    ) -> std::result::Result<RecoveryCompletion, OperationNotStarted<Self>>
    where
        F: FnMut(Progress) + Send,
    {
        let mut last = None;
        let mut malformed = false;
        let result = self.core().handle.restart_with_progress(&mut |native| {
            deliver_progress(native, &mut last, &mut malformed, &mut callback);
        });
        if let Err(error) = result {
            if operation_not_started(error) {
                return Err(OperationNotStarted::new(self, map_sys_error(error)));
            }
            let outcome = RecoveryOutcome {
                shutdown: self.shutdown.clone(),
                restart: Some(OperationOutcome::Failed(map_operation_error(
                    error,
                    Operation::Restart,
                ))),
            };
            return Ok(RecoveryCompletion {
                core: self.take_core(),
                outcome,
            });
        }
        let restart = if malformed {
            OperationOutcome::Failed(malformed_progress_error())
        } else {
            OperationOutcome::Succeeded
        };
        let outcome = RecoveryOutcome {
            shutdown: self.shutdown.clone(),
            restart: Some(restart),
        };
        Ok(RecoveryCompletion {
            core: self.take_core(),
            outcome,
        })
    }

    /// Explicitly opts out of automatic restart.
    #[must_use]
    pub fn leave_stopped(mut self) -> RecoveryCompletion {
        let outcome = RecoveryOutcome {
            shutdown: self.shutdown.clone(),
            restart: None,
        };
        RecoveryCompletion {
            core: self.take_core(),
            outcome,
        }
    }
}

impl Drop for RestartPending {
    fn drop(&mut self) {
        if self.armed {
            if let Some(core) = self.core.take() {
                let _ = core.handle.restart();
                drop(core);
            }
            self.armed = false;
        }
    }
}

/// Completed recovery state with retained results and post-operation reporting.
pub struct RecoveryCompletion {
    core: SessionCore,
    outcome: RecoveryOutcome,
}

impl RecoveryCompletion {
    /// Returns both retained operation outcomes.
    #[must_use]
    pub const fn outcome(&self) -> &RecoveryOutcome {
        &self.outcome
    }

    /// Takes a post-operation affected-application snapshot.
    pub fn affected_applications(&mut self) -> Result<AffectedApplications> {
        self.core.affected_applications()
    }

    /// Ends the session and returns both retained operation outcomes.
    ///
    /// If ending fails, destruction retries the native end once.
    pub fn end(self) -> Result<RecoveryOutcome> {
        let outcome = self.outcome;
        self.core.end()?;
        Ok(outcome)
    }
}

/// A consuming operation that failed before native work began.
pub struct OperationNotStarted<T> {
    state: T,
    error: Error,
}

impl<T> OperationNotStarted<T> {
    pub(crate) fn new(state: T, error: Error) -> Self {
        Self { state, error }
    }

    /// Returns the error.
    #[must_use]
    pub const fn error(&self) -> &Error {
        &self.error
    }

    /// Returns the original state.
    #[must_use]
    pub const fn state(&self) -> &T {
        &self.state
    }

    /// Separates the original state and error.
    #[must_use]
    pub fn into_parts(self) -> (T, Error) {
        (self.state, self.error)
    }
}

impl<T> fmt::Debug for OperationNotStarted<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OperationNotStarted")
            .field("state", &std::any::type_name::<T>())
            .field("error", &self.error)
            .finish()
    }
}

/// A secondary-installer view of an existing session.
///
/// This role can only register resources, inspect its key, and end.
pub struct JoinedSession {
    core: SessionCore,
}

impl JoinedSession {
    /// Joins a session created by a primary installer.
    pub fn join(key: &SessionKey) -> Result<Self> {
        SessionCore::join(key).map(|core| Self { core })
    }

    /// Returns the key used by this joined session.
    #[must_use]
    pub fn session_key(&self) -> &SessionKey {
        &self.core.key
    }

    /// Registers a mixed collection in one native call.
    pub fn register_resources(&mut self, resources: &ResourceBatch) -> Result<()> {
        self.core.register_resources(resources)
    }

    /// Registers file paths.
    pub fn register_files<I, P>(&mut self, files: I) -> Result<()>
    where
        I: IntoIterator<Item = P>,
        P: AsRef<Path>,
    {
        let mut resources = ResourceBatch::new();
        for file in files {
            resources.add_file(file.as_ref().to_path_buf());
        }
        self.core.register_resources(&resources)
    }

    /// Registers exact process identities from any iterator.
    pub fn register_processes<I, P>(&mut self, processes: I) -> Result<()>
    where
        I: IntoIterator<Item = P>,
        P: Borrow<ProcessIdentity>,
    {
        let mut resources = ResourceBatch::new();
        for process in processes {
            resources.add_process(*process.borrow());
        }
        self.core.register_resources(&resources)
    }

    /// Registers Windows services by short name.
    pub fn register_services<I, S>(&mut self, services: I) -> Result<()>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut resources = ResourceBatch::new();
        for service in services {
            resources.add_service(service.as_ref().to_os_string());
        }
        self.core.register_resources(&resources)
    }

    /// Ends the joined handle.
    pub fn end(self) -> Result<()> {
        self.core.end()
    }
}

/// A weak capability that can cancel a blocking native operation.
#[derive(Clone)]
pub struct CancellationHandle {
    handle: Weak<SessionHandle>,
}

impl fmt::Debug for CancellationHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CancellationHandle")
            .finish_non_exhaustive()
    }
}

impl CancellationHandle {
    /// Requests cancellation without keeping an ended session alive.
    pub fn cancel(&self) -> Result<()> {
        let handle = self.handle.upgrade().ok_or_else(|| {
            Error::new(
                ErrorKind::SessionEnded,
                None,
                "the Restart Manager session has already ended",
            )
        })?;
        handle.cancel().map_err(map_sys_error)
    }
}

impl ProcessIdentity {
    /// Looks up a process creation time and builds a non-recyclable identity.
    pub fn from_pid(pid: u32) -> Result<Self> {
        if pid == 0 || pid == INVALID_NATIVE_ID {
            return Self::from_raw_parts(pid, 0);
        }
        sys::process_from_pid(pid)
            .map(process_from_raw)
            .map_err(map_sys_error)
    }

    /// Returns the current process identity.
    pub fn current() -> Result<Self> {
        sys::current_process()
            .map(process_from_raw)
            .map_err(map_sys_error)
    }
}

fn deliver_progress<F>(
    native: u32,
    last: &mut Option<Progress>,
    malformed: &mut bool,
    callback: &mut F,
) where
    F: FnMut(Progress),
{
    if *malformed {
        return;
    }
    let Some(progress) = Progress::try_from_native(native) else {
        *malformed = true;
        return;
    };
    if last.is_some_and(|previous| progress <= previous) {
        *malformed = true;
        return;
    }
    *last = Some(progress);
    callback(progress);
}

fn malformed_progress_error() -> Error {
    Error::new(
        ErrorKind::MalformedOsData,
        None,
        "Windows reported progress outside 0..=100 or not strictly increasing",
    )
}

fn operation_not_started(error: SysError) -> bool {
    matches!(
        error,
        SysError::CallbackInUse | SysError::UnsupportedPlatform
    )
}

fn validate_os_value(value: &OsStr, description: &'static str) -> Result<()> {
    if value.is_empty() {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            None,
            format!("{description} may not be empty"),
        ));
    }
    if value.to_string_lossy().contains('\0') {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            None,
            format!("{description} may not contain an embedded NUL"),
        ));
    }
    Ok(())
}

fn validate_and_absolute_file(path: &Path) -> Result<PathBuf> {
    validate_os_value(path.as_os_str(), "a file path")?;
    let path = std::path::absolute(path).map_err(|error| {
        Error::new(
            ErrorKind::InvalidInput,
            error.raw_os_error().map(|code| code as u32),
            "a file path could not be made absolute",
        )
    })?;
    if path.is_dir() {
        return Err(Error::new(
            ErrorKind::DirectoryNotSupported,
            None,
            "Restart Manager does not support directory resources",
        ));
    }
    Ok(path)
}

fn application_report(raw: RawAffectedApplications) -> AffectedApplications {
    let applications = raw
        .applications
        .into_iter()
        .map(|application| AffectedApplication {
            display_name: application.display_name,
            service_name: (!application.service_name.is_empty())
                .then_some(application.service_name),
            application_type: ApplicationType::from_raw(application.application_type),
            status: ApplicationStatus::from_bits_retain(application.status),
            restartable: application.restartable,
            process: (application.process.pid != 0 && application.process.pid != INVALID_NATIVE_ID)
                .then(|| process_from_raw(application.process)),
            terminal_session_id: (application.terminal_session_id != INVALID_NATIVE_ID)
                .then_some(application.terminal_session_id),
        })
        .collect();
    AffectedApplications {
        applications,
        reboot_reasons: RebootReasons::from_bits_retain(raw.reboot_reasons),
    }
}

fn raw_process(process: ProcessIdentity) -> RawUniqueProcess {
    RawUniqueProcess {
        pid: process.pid(),
        start_time: process.creation_time_100ns_since_1601(),
    }
}

fn process_from_raw(process: RawUniqueProcess) -> ProcessIdentity {
    ProcessIdentity::from_raw_parts_unchecked(process.pid, process.start_time)
}

fn raw_filter_target(target: &FilterTarget) -> RawFilterTarget {
    if let Some(path) = target.as_executable() {
        RawFilterTarget::Executable(path.to_path_buf())
    } else if let Some(process) = target.as_process() {
        RawFilterTarget::Process(raw_process(process))
    } else if let Some(service) = target.as_service() {
        RawFilterTarget::Service(service.to_os_string())
    } else {
        unreachable!("validated filter targets have exactly one identity")
    }
}

fn raw_filter_action(action: FilterAction) -> i32 {
    match action {
        FilterAction::PreventRestart => 1,
        FilterAction::PreventShutdown => 2,
    }
}

fn filter_from_raw(filter: RawFilter) -> Result<Filter> {
    let target = match filter.target {
        RawFilterTarget::Executable(path) => FilterTarget::from_raw_executable(path)?,
        RawFilterTarget::Process(process) => {
            if process.pid == 0 || process.pid == INVALID_NATIVE_ID {
                return Err(Error::new(
                    ErrorKind::MalformedOsData,
                    None,
                    "Windows returned an invalid process filter identity",
                ));
            }
            FilterTarget::from_raw_process(process_from_raw(process))
        }
        RawFilterTarget::Service(service) => FilterTarget::from_raw_service(service)?,
    };
    let action = match filter.action {
        1 => FilterAction::PreventRestart,
        2 => FilterAction::PreventShutdown,
        _ => {
            return Err(Error::new(
                ErrorKind::MalformedOsData,
                None,
                "Windows returned an unknown filter action",
            ));
        }
    };
    Ok(Filter { target, action })
}

#[derive(Clone, Copy)]
enum Operation {
    Register,
    Report,
    Shutdown,
    Restart,
    SetFilter,
    RemoveFilter,
    Filters,
    End,
}

fn map_join_error(error: SysError) -> Error {
    match error {
        SysError::Os(
            ERROR_INVALID_HANDLE | ERROR_BAD_ARGUMENTS | ERROR_SESSION_CREDENTIAL_CONFLICT,
        ) => Error::new(
            ErrorKind::InvalidSessionKey,
            error.raw_os_error(),
            "the Restart Manager session key was rejected",
        ),
        other => map_sys_error(other),
    }
}

fn map_operation_error(error: SysError, operation: Operation) -> Error {
    match (operation, error) {
        (Operation::Register | Operation::Report, SysError::Os(ERROR_DIRECTORY)) => Error::new(
            ErrorKind::DirectoryNotSupported,
            Some(ERROR_DIRECTORY),
            "Restart Manager does not support directory resources",
        ),
        (Operation::Shutdown, SysError::Os(ERROR_FAIL_NOACTION_REBOOT)) => Error::new(
            ErrorKind::RebootRequired,
            Some(ERROR_FAIL_NOACTION_REBOOT),
            "a system reboot is required before applications can be shut down",
        ),
        (Operation::Shutdown, SysError::Os(ERROR_FAIL_SHUTDOWN)) => Error::new(
            ErrorKind::ShutdownIncomplete,
            Some(ERROR_FAIL_SHUTDOWN),
            "one or more affected applications could not be shut down",
        ),
        (Operation::Restart, SysError::Os(ERROR_FAIL_RESTART)) => Error::new(
            ErrorKind::RestartIncomplete,
            Some(ERROR_FAIL_RESTART),
            "one or more stopped applications could not be restarted",
        ),
        (Operation::Restart, SysError::Os(ERROR_REQUEST_OUT_OF_SEQUENCE)) => Error::new(
            ErrorKind::OperationOutOfSequence,
            Some(ERROR_REQUEST_OUT_OF_SEQUENCE),
            "Restart Manager rejected an out-of-sequence restart",
        ),
        (Operation::RemoveFilter, SysError::Os(ERROR_FILE_NOT_FOUND)) => Error::new(
            ErrorKind::FilterNotFound,
            Some(ERROR_FILE_NOT_FOUND),
            "the selected Restart Manager filter does not exist",
        ),
        (_, other) => map_sys_error(other),
    }
}

pub(crate) fn map_sys_error(error: SysError) -> Error {
    match error {
        SysError::UnsupportedPlatform => Error::new(
            ErrorKind::UnsupportedPlatform,
            None,
            "Windows Restart Manager is unavailable on this platform",
        ),
        SysError::Os(ERROR_MAX_SESSIONS_REACHED) => Error::new(
            ErrorKind::SessionLimit,
            Some(ERROR_MAX_SESSIONS_REACHED),
            "the limit of 64 concurrent Restart Manager sessions was reached",
        ),
        SysError::Os(ERROR_CANCELLED) => Error::new(
            ErrorKind::Cancelled,
            Some(ERROR_CANCELLED),
            "the Restart Manager operation was cancelled",
        ),
        SysError::Os(ERROR_ACCESS_DENIED) => Error::new(
            ErrorKind::AccessDenied,
            Some(ERROR_ACCESS_DENIED),
            "Windows denied access to the requested resource",
        ),
        SysError::Os(ERROR_BAD_ARGUMENTS) => Error::new(
            ErrorKind::InvalidInput,
            Some(ERROR_BAD_ARGUMENTS),
            "Restart Manager rejected an invalid argument",
        ),
        SysError::Os(ERROR_INVALID_HANDLE) => Error::new(
            ErrorKind::SessionEnded,
            Some(ERROR_INVALID_HANDLE),
            "the Restart Manager session handle is no longer valid",
        ),
        SysError::Os(ERROR_OUTOFMEMORY) => Error::new(
            ErrorKind::OutOfMemory,
            Some(ERROR_OUTOFMEMORY),
            "Restart Manager could not allocate required memory",
        ),
        SysError::Os(ERROR_SEM_TIMEOUT | ERROR_WRITE_FAULT) => Error::new(
            ErrorKind::RegistryUnavailable,
            error.raw_os_error(),
            "Restart Manager could not access its registry state",
        ),
        SysError::Os(code) => Error::new(
            ErrorKind::Os,
            Some(code),
            format!("Restart Manager call failed (Win32 error {code})"),
        ),
        SysError::HResult(code) => {
            let kind = match code as u32 {
                0x8007_0057 => ErrorKind::InvalidInput,
                0x8007_000E => ErrorKind::OutOfMemory,
                _ => ErrorKind::Os,
            };
            Error::from_hresult(
                kind,
                code,
                format!(
                    "application restart call failed (HRESULT {:#010x})",
                    code as u32
                ),
            )
        }
        SysError::InvalidInput(detail) => Error::new(ErrorKind::InvalidInput, None, detail),
        SysError::CountOverflow => Error::new(
            ErrorKind::TooManyResources,
            None,
            "a native Restart Manager count would overflow",
        ),
        SysError::AllocationFailure => Error::new(
            ErrorKind::OutOfMemory,
            None,
            "memory allocation for a native result failed",
        ),
        SysError::CallbackInUse => Error::new(
            ErrorKind::CallbackInUse,
            None,
            "another progress callback operation is active in this process",
        ),
        SysError::DataChanged(code) => Error::new(
            ErrorKind::DataChanged,
            Some(code),
            "the Restart Manager list kept changing through all eight retries",
        ),
        SysError::MalformedOutput(detail) => Error::new(ErrorKind::MalformedOsData, None, detail),
        SysError::SessionEnded => Error::new(
            ErrorKind::SessionEnded,
            None,
            "the Restart Manager session has already ended",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn key_parse_is_dedicated_and_debug_is_redacted() {
        let key: SessionKey = "0123456789abcdefABCDEF0123456789".parse().unwrap();
        assert_eq!(key.as_str(), "0123456789abcdefABCDEF0123456789");
        assert_eq!(format!("{key:?}"), "SessionKey(\"<redacted>\")");
        assert!("short".parse::<SessionKey>().is_err());
        assert_eq!(
            key.clone().into_string(),
            "0123456789abcdefABCDEF0123456789"
        );
        assert_eq!(
            SessionKey::generated("invalid".to_owned())
                .unwrap_err()
                .kind(),
            ErrorKind::MalformedOsData
        );
    }

    #[test]
    fn operation_mapping_is_specific() {
        assert_eq!(
            map_operation_error(SysError::Os(351), Operation::Shutdown).kind(),
            ErrorKind::ShutdownIncomplete
        );
        assert_eq!(
            map_operation_error(SysError::Os(352), Operation::Restart).kind(),
            ErrorKind::RestartIncomplete
        );
        assert_eq!(
            map_operation_error(SysError::Os(267), Operation::Register).kind(),
            ErrorKind::DirectoryNotSupported
        );
        assert_eq!(
            map_operation_error(SysError::Os(350), Operation::Shutdown).kind(),
            ErrorKind::RebootRequired
        );
        assert_eq!(
            map_operation_error(SysError::Os(776), Operation::Restart).kind(),
            ErrorKind::OperationOutOfSequence
        );
        assert_eq!(
            map_operation_error(SysError::Os(2), Operation::RemoveFilter).kind(),
            ErrorKind::FilterNotFound
        );
        assert_eq!(
            map_sys_error(SysError::Os(121)).kind(),
            ErrorKind::RegistryUnavailable
        );
        assert_eq!(
            map_sys_error(SysError::Os(14)).kind(),
            ErrorKind::OutOfMemory
        );
        assert_eq!(
            map_sys_error(SysError::Os(160)).kind(),
            ErrorKind::InvalidInput
        );
        let cases = [
            (
                SysError::UnsupportedPlatform,
                ErrorKind::UnsupportedPlatform,
            ),
            (SysError::Os(353), ErrorKind::SessionLimit),
            (SysError::Os(1223), ErrorKind::Cancelled),
            (SysError::Os(5), ErrorKind::AccessDenied),
            (SysError::Os(6), ErrorKind::SessionEnded),
            (SysError::Os(29), ErrorKind::RegistryUnavailable),
            (SysError::Os(999), ErrorKind::Os),
            (SysError::InvalidInput("bad"), ErrorKind::InvalidInput),
            (SysError::CountOverflow, ErrorKind::TooManyResources),
            (SysError::AllocationFailure, ErrorKind::OutOfMemory),
            (SysError::CallbackInUse, ErrorKind::CallbackInUse),
            (SysError::DataChanged(234), ErrorKind::DataChanged),
            (
                SysError::MalformedOutput("bad output"),
                ErrorKind::MalformedOsData,
            ),
            (SysError::SessionEnded, ErrorKind::SessionEnded),
        ];
        for (error, kind) in cases {
            assert_eq!(map_sys_error(error).kind(), kind);
        }
        for code in [
            ERROR_INVALID_HANDLE,
            ERROR_BAD_ARGUMENTS,
            ERROR_SESSION_CREDENTIAL_CONFLICT,
        ] {
            assert_eq!(
                map_join_error(SysError::Os(code)).kind(),
                ErrorKind::InvalidSessionKey
            );
        }
        assert_eq!(map_join_error(SysError::Os(999)).kind(), ErrorKind::Os);
    }

    #[test]
    fn hresult_mapping_is_lossless() {
        let error = map_sys_error(SysError::HResult(0x8007_0057_u32 as i32));
        assert_eq!(error.kind(), ErrorKind::InvalidInput);
        assert_eq!(error.raw_hresult(), Some(0x8007_0057_u32 as i32));
        assert_eq!(error.raw_os_error(), None);
    }

    #[test]
    fn progress_rejects_duplicate_decrease_and_range() {
        let mut last = None;
        let mut malformed = false;
        let mut values = Vec::new();
        deliver_progress(10, &mut last, &mut malformed, &mut |p| values.push(p));
        deliver_progress(10, &mut last, &mut malformed, &mut |p| values.push(p));
        deliver_progress(20, &mut last, &mut malformed, &mut |p| values.push(p));
        assert!(malformed);
        assert_eq!(values.len(), 1);
        deliver_progress(101, &mut last, &mut malformed, &mut |p| values.push(p));
        let mut fresh_last = None;
        let mut fresh_malformed = false;
        deliver_progress(101, &mut fresh_last, &mut fresh_malformed, &mut |_| {});
        assert!(fresh_malformed);
        assert_eq!(
            malformed_progress_error().kind(),
            ErrorKind::MalformedOsData
        );
        assert!(operation_not_started(SysError::UnsupportedPlatform));
        assert!(!operation_not_started(SysError::Os(5)));
    }

    #[cfg(windows)]
    #[test]
    #[allow(clippy::result_large_err)]
    fn callback_lease_failure_returns_owned_state_before_shutdown_or_restart() {
        let session = RestartSession::new().unwrap();
        let result = sys::with_callback_lease_for_test(|| {
            session.shutdown_with_progress(ShutdownOptions::default(), |_| {})
        });
        let not_started = match result {
            Err(not_started) => not_started,
            Ok(_) => panic!("shutdown unexpectedly acquired the callback lease"),
        };
        assert_eq!(not_started.error().kind(), ErrorKind::CallbackInUse);
        assert!(not_started.state().session_key().as_str().len() == 32);
        assert!(format!("{not_started:?}").contains("OperationNotStarted"));
        let (session, _) = not_started.into_parts();

        let pending = session.shutdown();
        let result = sys::with_callback_lease_for_test(|| pending.restart_with_progress(|_| {}));
        let not_started = match result {
            Err(not_started) => not_started,
            Ok(_) => panic!("restart unexpectedly acquired the callback lease"),
        };
        assert_eq!(not_started.error().kind(), ErrorKind::CallbackInUse);
        let (pending, _) = not_started.into_parts();
        pending.leave_stopped().end().unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn progress_paths_and_pending_drop_cover_recovery_lifecycle() {
        let _test_guard = sys::serialize_callback_test();
        let mut session = RestartSession::new().unwrap();
        let process = ProcessIdentity::current().unwrap();
        session.register_processes([process]).unwrap();
        session
            .set_filter(
                &FilterTarget::process(process),
                FilterAction::PreventShutdown,
            )
            .unwrap();
        let mut shutdown_progress = Vec::new();
        let pending = session
            .shutdown_with_progress(ShutdownOptions::default(), |progress| {
                shutdown_progress.push(progress);
            })
            .unwrap();
        assert!(!shutdown_progress.is_empty());
        let mut completion = pending.restart_with_progress(|_| {}).unwrap();
        completion.affected_applications().unwrap();
        completion.end().unwrap();

        RestartSession::new()
            .unwrap()
            .shutdown()
            .leave_stopped()
            .end()
            .unwrap();
        drop(RestartSession::new().unwrap().shutdown());
    }

    #[test]
    fn report_and_filter_raw_conversions_cover_every_variant() {
        let process = RawUniqueProcess {
            pid: 17,
            start_time: 23,
        };
        let report = application_report(RawAffectedApplications {
            applications: vec![sys::RawApplication {
                display_name: OsString::from("display"),
                service_name: OsString::from("service"),
                application_type: 3,
                status: 0x21,
                restartable: true,
                process,
                terminal_session_id: 4,
            }],
            reboot_reasons: 2,
        });
        assert_eq!(
            report.applications()[0].display_name(),
            OsStr::new("display")
        );
        assert_eq!(
            report.applications()[0].service_name(),
            Some(OsStr::new("service"))
        );
        assert_eq!(report.applications()[0].process().unwrap().pid(), 17);
        assert_eq!(report.applications()[0].terminal_session_id(), Some(4));

        for target in [
            RawFilterTarget::Executable(PathBuf::from("absolute.exe")),
            RawFilterTarget::Process(process),
            RawFilterTarget::Service(OsString::from("EventLog")),
        ] {
            let filter = filter_from_raw(RawFilter { target, action: 1 }).unwrap();
            assert_eq!(filter.action(), FilterAction::PreventRestart);
        }
        assert_eq!(
            filter_from_raw(RawFilter {
                target: RawFilterTarget::Service(OsString::from("EventLog")),
                action: 99,
            })
            .unwrap_err()
            .kind(),
            ErrorKind::MalformedOsData
        );
        assert_eq!(
            filter_from_raw(RawFilter {
                target: RawFilterTarget::Process(RawUniqueProcess {
                    pid: 0,
                    start_time: 0,
                }),
                action: 1,
            })
            .unwrap_err()
            .kind(),
            ErrorKind::MalformedOsData
        );

        let report = application_report(RawAffectedApplications {
            applications: vec![sys::RawApplication {
                display_name: OsString::new(),
                service_name: OsString::new(),
                application_type: 0,
                status: 0,
                restartable: false,
                process: RawUniqueProcess {
                    pid: INVALID_NATIVE_ID,
                    start_time: 0,
                },
                terminal_session_id: INVALID_NATIVE_ID,
            }],
            reboot_reasons: 0,
        });
        assert!(report.applications()[0].service_name().is_none());
        assert!(report.applications()[0].process().is_none());
        assert!(report.applications()[0].terminal_session_id().is_none());
    }

    #[cfg(windows)]
    #[test]
    fn public_input_validation_and_debug_paths_are_reached() {
        assert!(ProcessIdentity::from_pid(0).is_err());
        assert!(ProcessIdentity::from_pid(INVALID_NATIVE_ID).is_err());

        let mut session = RestartSession::new().unwrap();
        assert_eq!(
            session.register_services([""]).unwrap_err().kind(),
            ErrorKind::InvalidInput
        );
        assert_eq!(
            session
                .register_services(["bad\0service"])
                .unwrap_err()
                .kind(),
            ErrorKind::InvalidInput
        );
        assert_eq!(
            session.register_files([""]).unwrap_err().kind(),
            ErrorKind::InvalidInput
        );
        let cancellation = session.cancellation_handle();
        assert!(format!("{cancellation:?}").contains("CancellationHandle"));
        session.end().unwrap();
        assert_eq!(
            cancellation.cancel().unwrap_err().kind(),
            ErrorKind::SessionEnded
        );
    }
}
