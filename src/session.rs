//! Role-aware session lifecycle and all public session behavior.

use std::ffi::OsStr;
use std::fmt;
use std::path::Path;
use std::str::FromStr;
use std::sync::{Arc, Weak};

use crate::application::{
    AffectedApplication, AffectedApplications, ApplicationStatus, ApplicationType, RebootReasons,
    UniqueProcess,
};
use crate::error::{Error, ErrorKind, Result};
use crate::filter::{Filter, FilterAction, FilterTarget};
use crate::resource::ResourceSet;
use crate::shutdown::{Progress, ShutdownOptions};
use crate::sys::{
    self, RawAffectedApplications, RawFilter, RawFilterTarget, RawUniqueProcess, SessionHandle,
    SysError,
};

const INVALID_NATIVE_ID: u32 = u32::MAX;
const ERROR_ACCESS_DENIED: u32 = 5;
const ERROR_INVALID_HANDLE: u32 = 6;
const ERROR_BAD_ARGUMENTS: u32 = 160;
const ERROR_MAX_SESSIONS_REACHED: u32 = 353;
const ERROR_SESSION_CREDENTIAL_CONFLICT: u32 = 1219;
const ERROR_CANCELLED: u32 = 1223;

/// A validated, 32-character ASCII hexadecimal Restart Manager session key.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct SessionKey(String);

impl SessionKey {
    /// Returns the validated key for explicit cross-process transfer.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
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

impl fmt::Display for SessionKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl FromStr for SessionKey {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        if is_valid_key(value) {
            Ok(Self(value.to_owned()))
        } else {
            Err(Error::new(
                ErrorKind::InvalidSessionKey,
                None,
                "a session key must contain exactly 32 ASCII hexadecimal characters",
            ))
        }
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

    fn register_resources(&mut self, resources: &ResourceSet) -> Result<()> {
        let files = resources.files().map(Path::to_path_buf).collect::<Vec<_>>();
        let processes = resources
            .processes()
            .iter()
            .copied()
            .map(raw_process)
            .collect::<Vec<_>>();
        let services = resources
            .services()
            .map(OsStr::to_os_string)
            .collect::<Vec<_>>();
        self.handle
            .register_resources(&files, &processes, &services)
            .map_err(map_sys_error)
    }

    fn affected_applications(&mut self) -> Result<AffectedApplications> {
        self.handle
            .affected_applications()
            .map_err(map_sys_error)
            .map(application_report)
    }

    fn end(self) -> Result<()> {
        self.handle.end().map_err(map_sys_error)
    }
}

/// A primary-installer Restart Manager session.
///
/// Only this role exposes shutdown, restart, cancellation, and filter
/// operations. The session ends automatically on drop.
pub struct RestartSession {
    core: SessionCore,
}

impl RestartSession {
    /// Starts a new primary-installer session.
    pub fn new() -> Result<Self> {
        SessionCore::start().map(|core| Self { core })
    }

    /// Returns the key a secondary installer can pass to [`JoinedSession::join`].
    #[must_use]
    pub fn session_key(&self) -> &SessionKey {
        &self.core.key
    }

    /// Registers a mixed collection in one `RmRegisterResources` call.
    pub fn register_resources(&mut self, resources: &ResourceSet) -> Result<()> {
        self.core.register_resources(resources)
    }

    /// Registers file paths. Directories are not supported.
    ///
    /// Relative paths become absolute without existence checks or
    /// canonicalization.
    pub fn register_files<I, P>(&mut self, files: I) -> Result<()>
    where
        I: IntoIterator<Item = P>,
        P: AsRef<Path>,
    {
        let mut resources = ResourceSet::new();
        for file in files {
            resources.add_file(file.as_ref().to_path_buf());
        }
        self.core.register_resources(&resources)
    }

    /// Registers exact process identities.
    pub fn register_processes(&mut self, processes: &[UniqueProcess]) -> Result<()> {
        let mut resources = ResourceSet::new();
        for process in processes {
            resources.add_process(*process);
        }
        self.core.register_resources(&resources)
    }

    /// Registers Windows services by short name.
    pub fn register_services<I, S>(&mut self, services: I) -> Result<()>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut resources = ResourceSet::new();
        for service in services {
            resources.add_service(service.as_ref().to_os_string());
        }
        self.core.register_resources(&resources)
    }

    /// Takes a reusable snapshot of affected applications and reboot reasons.
    pub fn affected_applications(&mut self) -> Result<AffectedApplications> {
        self.core.affected_applications()
    }

    /// Creates a weak, thread-safe capability for cancelling blocking work.
    ///
    /// Keeping the returned value alive does not keep the native session slot
    /// occupied.
    #[must_use]
    pub fn cancellation_handle(&self) -> CancellationHandle {
        CancellationHandle {
            handle: Arc::downgrade(&self.core.handle),
        }
    }

    /// Gracefully shuts down affected applications.
    pub fn shutdown(&mut self) -> Result<()> {
        self.shutdown_with_options(ShutdownOptions::default())
    }

    /// Shuts down affected applications with explicit options.
    pub fn shutdown_with_options(&mut self, options: ShutdownOptions) -> Result<()> {
        self.core
            .handle
            .shutdown(options.native_flags())
            .map_err(map_sys_error)
    }

    /// Shuts down while reporting bounded, non-decreasing progress.
    ///
    /// Only one callback-bearing Restart Manager operation may run in a
    /// process. A concurrent attempt returns [`ErrorKind::CallbackInUse`].
    /// Callback panics are resumed after the native call returns.
    pub fn shutdown_with_progress<F>(
        &mut self,
        options: ShutdownOptions,
        mut callback: F,
    ) -> Result<()>
    where
        F: FnMut(Progress) + Send,
    {
        let mut last = 0;
        let mut adapter = |native: u32| {
            last = last.max(native.min(100));
            callback(Progress::from_native(last));
        };
        self.core
            .handle
            .shutdown_with_progress(options.native_flags(), &mut adapter)
            .map_err(map_sys_error)
    }

    /// Restarts applications that Restart Manager stopped and can restart.
    pub fn restart(&mut self) -> Result<()> {
        self.core.handle.restart().map_err(map_sys_error)
    }

    /// Restarts affected applications while reporting bounded, non-decreasing progress.
    pub fn restart_with_progress<F>(&mut self, mut callback: F) -> Result<()>
    where
        F: FnMut(Progress) + Send,
    {
        let mut last = 0;
        let mut adapter = |native: u32| {
            last = last.max(native.min(100));
            callback(Progress::from_native(last));
        };
        self.core
            .handle
            .restart_with_progress(&mut adapter)
            .map_err(map_sys_error)
    }

    /// Adds or replaces a restart/shutdown filter.
    pub fn set_filter(&mut self, target: FilterTarget, action: FilterAction) -> Result<()> {
        self.core
            .handle
            .add_filter(&raw_filter_target(&target), raw_filter_action(action))
            .map_err(map_sys_error)
    }

    /// Removes a filter from the selected target.
    pub fn remove_filter(&mut self, target: &FilterTarget) -> Result<()> {
        self.core
            .handle
            .remove_filter(&raw_filter_target(target))
            .map_err(map_sys_error)
    }

    /// Lists filters configured by this primary installer.
    pub fn filters(&mut self) -> Result<Vec<Filter>> {
        self.core
            .handle
            .filters()
            .map_err(map_sys_error)?
            .into_iter()
            .map(filter_from_raw)
            .collect()
    }

    /// Ends the session now and reports `RmEndSession` failures.
    ///
    /// Dropping without calling this method remains the normal RAII path.
    pub fn end(self) -> Result<()> {
        self.core.end()
    }
}

/// A secondary-installer view of an existing session.
///
/// This role can register resources and query affected applications, but it
/// cannot shut down, restart, cancel, or manipulate filters.
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

    /// Registers a mixed collection in one `RmRegisterResources` call.
    pub fn register_resources(&mut self, resources: &ResourceSet) -> Result<()> {
        self.core.register_resources(resources)
    }

    /// Registers file paths. Directories are not supported.
    pub fn register_files<I, P>(&mut self, files: I) -> Result<()>
    where
        I: IntoIterator<Item = P>,
        P: AsRef<Path>,
    {
        let mut resources = ResourceSet::new();
        for file in files {
            resources.add_file(file.as_ref().to_path_buf());
        }
        self.core.register_resources(&resources)
    }

    /// Registers exact process identities.
    pub fn register_processes(&mut self, processes: &[UniqueProcess]) -> Result<()> {
        let mut resources = ResourceSet::new();
        for process in processes {
            resources.add_process(*process);
        }
        self.core.register_resources(&resources)
    }

    /// Registers Windows services by short name.
    pub fn register_services<I, S>(&mut self, services: I) -> Result<()>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut resources = ResourceSet::new();
        for service in services {
            resources.add_service(service.as_ref().to_os_string());
        }
        self.core.register_resources(&resources)
    }

    /// Takes a reusable snapshot of affected applications and reboot reasons.
    pub fn affected_applications(&mut self) -> Result<AffectedApplications> {
        self.core.affected_applications()
    }

    /// Ends the joined handle now and reports `RmEndSession` failures.
    pub fn end(self) -> Result<()> {
        self.core.end()
    }
}

/// A weak capability that can cancel a blocking shutdown or restart.
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
    /// Requests cancellation without keeping an otherwise dropped session alive.
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

impl UniqueProcess {
    /// Looks up a process creation time and builds a non-recyclable identity.
    pub fn from_pid(pid: u32) -> Result<Self> {
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
            process: (application.process.pid != INVALID_NATIVE_ID)
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

fn raw_process(process: UniqueProcess) -> RawUniqueProcess {
    RawUniqueProcess {
        pid: process.pid(),
        start_time: process.start_time(),
    }
}

fn process_from_raw(process: RawUniqueProcess) -> UniqueProcess {
    UniqueProcess::from_parts(process.pid, process.start_time)
}

fn raw_filter_target(target: &FilterTarget) -> RawFilterTarget {
    match target {
        FilterTarget::Executable(path) => RawFilterTarget::Executable(path.clone()),
        FilterTarget::Process(process) => RawFilterTarget::Process(raw_process(*process)),
        FilterTarget::Service(name) => RawFilterTarget::Service(name.clone()),
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
        RawFilterTarget::Executable(path) => FilterTarget::Executable(path),
        RawFilterTarget::Process(process) => FilterTarget::Process(process_from_raw(process)),
        RawFilterTarget::Service(service) => FilterTarget::Service(service),
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

fn map_join_error(error: SysError) -> Error {
    match error {
        SysError::Os(code)
            if code == ERROR_INVALID_HANDLE
                || code == ERROR_BAD_ARGUMENTS
                || code == ERROR_SESSION_CREDENTIAL_CONFLICT =>
        {
            Error::new(
                ErrorKind::InvalidSessionKey,
                Some(code),
                format!("the Restart Manager session key was rejected (Win32 error {code})"),
            )
        }
        other => map_sys_error(other),
    }
}

fn map_sys_error(error: SysError) -> Error {
    match error {
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
        SysError::Os(code) => Error::new(
            ErrorKind::Os,
            Some(code),
            format!("Restart Manager call failed (Win32 error {code})"),
        ),
        SysError::InvalidInput(detail) => Error::new(ErrorKind::InvalidInput, None, detail),
        SysError::CountOverflow => Error::new(
            ErrorKind::TooManyResources,
            None,
            "a native Restart Manager count or allocation would overflow",
        ),
        SysError::CallbackInUse => Error::new(
            ErrorKind::CallbackInUse,
            None,
            "another progress callback operation is already active in this process",
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
    use std::path::PathBuf;

    #[test]
    fn session_key_validates_ascii_hex_and_redacts_debug() {
        let key: SessionKey = "0123456789abcdefABCDEF0123456789".parse().unwrap();
        assert_eq!(key.as_str(), "0123456789abcdefABCDEF0123456789");
        assert_eq!(format!("{key:?}"), "SessionKey(\"<redacted>\")");
        assert_eq!(key.to_string(), key.as_str());
        assert!("0123".parse::<SessionKey>().is_err());
        assert!(
            "g123456789abcdefABCDEF0123456789"
                .parse::<SessionKey>()
                .is_err()
        );
        assert!(
            "é123456789abcdefABCDEF012345678"
                .parse::<SessionKey>()
                .is_err()
        );
    }

    #[test]
    fn error_mapping_preserves_kind_and_raw_code() {
        let error = map_sys_error(SysError::Os(ERROR_ACCESS_DENIED));
        assert_eq!(error.kind(), ErrorKind::AccessDenied);
        assert_eq!(error.raw_os_error(), Some(ERROR_ACCESS_DENIED));

        let error = map_sys_error(SysError::CallbackInUse);
        assert_eq!(error.kind(), ErrorKind::CallbackInUse);
        assert_eq!(error.raw_os_error(), None);

        let cases = [
            (
                SysError::Os(ERROR_MAX_SESSIONS_REACHED),
                ErrorKind::SessionLimit,
            ),
            (SysError::Os(ERROR_CANCELLED), ErrorKind::Cancelled),
            (SysError::Os(999), ErrorKind::Os),
            (SysError::InvalidInput("bad"), ErrorKind::InvalidInput),
            (SysError::CountOverflow, ErrorKind::TooManyResources),
            (SysError::DataChanged(234), ErrorKind::DataChanged),
            (
                SysError::MalformedOutput("bad buffer"),
                ErrorKind::MalformedOsData,
            ),
            (SysError::SessionEnded, ErrorKind::SessionEnded),
        ];
        for (input, expected) in cases {
            assert_eq!(map_sys_error(input).kind(), expected);
        }
        assert_eq!(
            map_join_error(SysError::Os(ERROR_INVALID_HANDLE)).kind(),
            ErrorKind::InvalidSessionKey
        );
        assert_eq!(
            map_join_error(SysError::Os(ERROR_BAD_ARGUMENTS)).kind(),
            ErrorKind::InvalidSessionKey
        );
        assert_eq!(
            map_join_error(SysError::Os(ERROR_SESSION_CREDENTIAL_CONFLICT)).kind(),
            ErrorKind::InvalidSessionKey
        );
        assert_eq!(map_join_error(SysError::Os(999)).kind(), ErrorKind::Os);
    }

    #[test]
    fn invalid_native_ids_become_none() {
        let report = application_report(RawAffectedApplications {
            applications: vec![sys::RawApplication {
                display_name: OsString::from("demo"),
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
        assert!(report.applications()[0].process().is_none());
        assert!(report.applications()[0].terminal_session_id().is_none());
    }

    #[test]
    fn raw_domain_conversions_cover_filter_targets_and_actions() {
        let process = UniqueProcess::from_parts(5, 6);
        assert_eq!(process_from_raw(raw_process(process)), process);
        assert!(matches!(
            raw_filter_target(&FilterTarget::Executable(PathBuf::from("demo.exe"))),
            RawFilterTarget::Executable(_)
        ));
        assert!(matches!(
            raw_filter_target(&FilterTarget::Process(process)),
            RawFilterTarget::Process(_)
        ));
        assert!(matches!(
            raw_filter_target(&FilterTarget::Service(OsString::from("EventLog"))),
            RawFilterTarget::Service(_)
        ));
        assert_eq!(raw_filter_action(FilterAction::PreventRestart), 1);
        assert_eq!(raw_filter_action(FilterAction::PreventShutdown), 2);

        let targets = [
            RawFilterTarget::Executable(PathBuf::from("demo.exe")),
            RawFilterTarget::Process(raw_process(process)),
            RawFilterTarget::Service(OsString::from("EventLog")),
        ];
        for target in targets {
            let filter = filter_from_raw(RawFilter { target, action: 2 }).unwrap();
            assert_eq!(filter.action(), FilterAction::PreventShutdown);
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

        let cancellation = CancellationHandle {
            handle: Weak::new(),
        };
        assert!(format!("{cancellation:?}").starts_with("CancellationHandle"));
        assert_eq!(
            cancellation.cancel().unwrap_err().kind(),
            ErrorKind::SessionEnded
        );
    }
}
