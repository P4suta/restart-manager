//! Private, reviewed boundary around `windows-sys`.

#![allow(unsafe_code)]

use std::any::Any;
use std::ffi::{OsStr, OsString};
use std::mem::{self, MaybeUninit};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::{Mutex, TryLockError};

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_MORE_DATA, ERROR_SUCCESS, FILETIME, GetLastError, HANDLE,
};
use windows_sys::Win32::System::RestartManager::{
    CCH_RM_SESSION_KEY, RM_FILTER_INFO, RM_PROCESS_INFO, RM_UNIQUE_PROCESS,
    RM_WRITE_STATUS_CALLBACK, RmAddFilter, RmCancelCurrentTask, RmEndSession,
    RmFilterTriggerFile as RM_FILTER_TRIGGER_FILE,
    RmFilterTriggerProcess as RM_FILTER_TRIGGER_PROCESS,
    RmFilterTriggerService as RM_FILTER_TRIGGER_SERVICE, RmGetFilterList, RmGetList, RmJoinSession,
    RmNoRestart, RmNoShutdown, RmRegisterResources, RmRemoveFilter, RmRestart, RmShutdown,
    RmStartSession,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcessId, GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
};

const MAX_LIST_ATTEMPTS: usize = 8;

pub(crate) type SysResult<T> = std::result::Result<T, SysError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SysError {
    Os(u32),
    InvalidInput(&'static str),
    CountOverflow,
    CallbackInUse,
    DataChanged(u32),
    MalformedOutput(&'static str),
    SessionEnded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RawUniqueProcess {
    pub(crate) pid: u32,
    pub(crate) start_time: u64,
}

#[derive(Debug)]
pub(crate) struct RawApplication {
    pub(crate) display_name: OsString,
    pub(crate) service_name: OsString,
    pub(crate) application_type: i32,
    pub(crate) status: u32,
    pub(crate) restartable: bool,
    pub(crate) process: RawUniqueProcess,
    pub(crate) terminal_session_id: u32,
}

#[derive(Debug)]
pub(crate) struct RawAffectedApplications {
    pub(crate) applications: Vec<RawApplication>,
    pub(crate) reboot_reasons: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RawFilterTarget {
    Executable(PathBuf),
    Process(RawUniqueProcess),
    Service(OsString),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RawFilter {
    pub(crate) target: RawFilterTarget,
    pub(crate) action: i32,
}

/// Owns one native session handle and calls `RmEndSession` exactly once.
pub(crate) struct SessionHandle {
    raw: u32,
    ended: Mutex<bool>,
}

impl SessionHandle {
    pub(crate) fn start() -> SysResult<(Self, String)> {
        let mut raw = 0;
        let mut key = [0_u16; CCH_RM_SESSION_KEY as usize + 1];
        // SAFETY: both pointers refer to writable values of the documented size.
        let code = unsafe { RmStartSession(&mut raw, 0, key.as_mut_ptr()) };
        check(code)?;
        let key = decode_fixed(&key)?
            .into_string()
            .map_err(|_| SysError::MalformedOutput("session key was not valid Unicode"))?;
        Ok((
            Self {
                raw,
                ended: Mutex::new(false),
            },
            key,
        ))
    }

    pub(crate) fn join(key: &str) -> SysResult<Self> {
        let key = WideString::new(OsStr::new(key))?;
        let mut raw = 0;
        // SAFETY: the output pointer is valid and `key` is NUL-terminated.
        let code = unsafe { RmJoinSession(&mut raw, key.as_ptr()) };
        check(code)?;
        Ok(Self {
            raw,
            ended: Mutex::new(false),
        })
    }

    pub(crate) fn register_resources(
        &self,
        files: &[PathBuf],
        processes: &[RawUniqueProcess],
        services: &[OsString],
    ) -> SysResult<()> {
        let file_count = count(files.len())?;
        let process_count = count(processes.len())?;
        let service_count = count(services.len())?;

        let file_strings = files
            .iter()
            .map(|path| absolute_file(path).and_then(|path| WideString::new(path.as_os_str())))
            .collect::<SysResult<Vec<_>>>()?;
        let service_strings = services
            .iter()
            .map(|name| WideString::new(name))
            .collect::<SysResult<Vec<_>>>()?;
        let file_ptrs = file_strings
            .iter()
            .map(WideString::as_ptr)
            .collect::<Vec<_>>();
        let service_ptrs = service_strings
            .iter()
            .map(WideString::as_ptr)
            .collect::<Vec<_>>();
        let native_processes = processes
            .iter()
            .copied()
            .map(native_unique_process)
            .collect::<Vec<_>>();

        // SAFETY: all counts match their buffers; strings remain alive and
        // NUL-terminated for the duration of the call.
        let code = unsafe {
            RmRegisterResources(
                self.raw,
                file_count,
                pointer_or_null(&file_ptrs),
                process_count,
                pointer_or_null(&native_processes),
                service_count,
                pointer_or_null(&service_ptrs),
            )
        };
        check(code)
    }

    pub(crate) fn affected_applications(&self) -> SysResult<RawAffectedApplications> {
        let mut needed = 0;
        let mut supplied = 0;
        let mut reboot_reasons = 0;
        // SAFETY: output pointers are valid; a null data buffer is required for
        // the size probe when `supplied` is zero.
        let first = unsafe {
            RmGetList(
                self.raw,
                &mut needed,
                &mut supplied,
                ptr::null_mut(),
                &mut reboot_reasons,
            )
        };
        if first == ERROR_SUCCESS {
            return Ok(RawAffectedApplications {
                applications: Vec::new(),
                reboot_reasons,
            });
        }
        if first != ERROR_MORE_DATA {
            return Err(SysError::Os(first));
        }

        for _ in 0..MAX_LIST_ATTEMPTS {
            let capacity = usize::try_from(needed).map_err(|_| SysError::CountOverflow)?;
            let mut buffer = Vec::new();
            buffer
                .try_reserve_exact(capacity)
                .map_err(|_| SysError::CountOverflow)?;
            buffer.resize(capacity, RM_PROCESS_INFO::default());
            supplied = needed;

            // SAFETY: `buffer` contains `supplied` initialized entries and all
            // output pointers are valid for the duration of the call.
            let code = unsafe {
                RmGetList(
                    self.raw,
                    &mut needed,
                    &mut supplied,
                    buffer.as_mut_ptr(),
                    &mut reboot_reasons,
                )
            };
            if code == ERROR_MORE_DATA {
                continue;
            }
            check(code)?;
            let returned = usize::try_from(supplied).map_err(|_| SysError::CountOverflow)?;
            if returned > buffer.len() {
                return Err(SysError::MalformedOutput(
                    "RmGetList returned more entries than the supplied buffer",
                ));
            }
            buffer.truncate(returned);
            let applications = buffer
                .iter()
                .map(raw_application)
                .collect::<SysResult<Vec<_>>>()?;
            return Ok(RawAffectedApplications {
                applications,
                reboot_reasons,
            });
        }

        Err(SysError::DataChanged(ERROR_MORE_DATA))
    }

    pub(crate) fn add_filter(&self, target: &RawFilterTarget, action: i32) -> SysResult<()> {
        if action != RmNoRestart && action != RmNoShutdown {
            return Err(SysError::InvalidInput("invalid filter action"));
        }
        let mut filename = None;
        let mut process = None;
        let mut service = None;
        match target {
            RawFilterTarget::Executable(path) => {
                let path = absolute_file(path)?;
                filename = Some(WideString::new(path.as_os_str())?);
            }
            RawFilterTarget::Process(value) => process = Some(native_unique_process(*value)),
            RawFilterTarget::Service(name) => service = Some(WideString::new(name)?),
        }
        // SAFETY: exactly one target is populated, all optional pointers are
        // either null or refer to live, correctly shaped values.
        let code = unsafe {
            RmAddFilter(
                self.raw,
                filename.as_ref().map_or(ptr::null(), WideString::as_ptr),
                process.as_ref().map_or(ptr::null(), |value| value),
                service.as_ref().map_or(ptr::null(), WideString::as_ptr),
                action,
            )
        };
        check(code)
    }

    pub(crate) fn remove_filter(&self, target: &RawFilterTarget) -> SysResult<()> {
        let mut filename = None;
        let mut process = None;
        let mut service = None;
        match target {
            RawFilterTarget::Executable(path) => {
                let path = absolute_file(path)?;
                filename = Some(WideString::new(path.as_os_str())?);
            }
            RawFilterTarget::Process(value) => process = Some(native_unique_process(*value)),
            RawFilterTarget::Service(name) => service = Some(WideString::new(name)?),
        }
        // SAFETY: exactly one target is populated and all non-null pointers
        // remain valid for the call.
        let code = unsafe {
            RmRemoveFilter(
                self.raw,
                filename.as_ref().map_or(ptr::null(), WideString::as_ptr),
                process.as_ref().map_or(ptr::null(), |value| value),
                service.as_ref().map_or(ptr::null(), WideString::as_ptr),
            )
        };
        check(code)
    }

    pub(crate) fn filters(&self) -> SysResult<Vec<RawFilter>> {
        get_filters(self.raw)
    }

    pub(crate) fn shutdown(&self, flags: u32) -> SysResult<()> {
        // SAFETY: `self.raw` remains owned for this synchronous call.
        check(unsafe { RmShutdown(self.raw, flags, None) })
    }

    pub(crate) fn shutdown_with_progress<F>(&self, flags: u32, callback: &mut F) -> SysResult<()>
    where
        F: FnMut(u32) + Send,
    {
        run_with_callback(callback, |native| {
            // SAFETY: the callback lease and session handle outlive this call.
            unsafe { RmShutdown(self.raw, flags, native) }
        })
    }

    pub(crate) fn restart(&self) -> SysResult<()> {
        // SAFETY: `self.raw` remains owned for this synchronous call.
        check(unsafe { RmRestart(self.raw, 0, None) })
    }

    pub(crate) fn restart_with_progress<F>(&self, callback: &mut F) -> SysResult<()>
    where
        F: FnMut(u32) + Send,
    {
        run_with_callback(callback, |native| {
            // SAFETY: the callback lease and session handle outlive this call.
            unsafe { RmRestart(self.raw, 0, native) }
        })
    }

    pub(crate) fn cancel(&self) -> SysResult<()> {
        let ended = self.ended.lock().unwrap_or_else(|error| error.into_inner());
        if *ended {
            return Err(SysError::SessionEnded);
        }
        // Holding `ended` serializes this call with explicit end. An upgraded
        // `Arc` also prevents `Drop` until cancellation returns.
        check(unsafe { RmCancelCurrentTask(self.raw) })
    }

    pub(crate) fn end(&self) -> SysResult<()> {
        let mut ended = self.ended.lock().unwrap_or_else(|error| error.into_inner());
        if *ended {
            return Ok(());
        }
        *ended = true;
        // Marked before calling so the consuming explicit path and `Drop`
        // never invoke RmEndSession twice, even when Windows reports an error.
        check(unsafe { RmEndSession(self.raw) })
    }
}

impl Drop for SessionHandle {
    fn drop(&mut self) {
        let ended = self
            .ended
            .get_mut()
            .unwrap_or_else(|error| error.into_inner());
        if !*ended {
            *ended = true;
            // SAFETY: this is the final owner and the call is made exactly once.
            let _ = unsafe { RmEndSession(self.raw) };
        }
    }
}

pub(crate) fn current_process() -> SysResult<RawUniqueProcess> {
    // SAFETY: this function has no preconditions.
    process_from_pid(unsafe { GetCurrentProcessId() })
}

pub(crate) fn process_from_pid(pid: u32) -> SysResult<RawUniqueProcess> {
    // SAFETY: requesting query-only access for a caller-supplied PID.
    let raw = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if raw.is_null() {
        // SAFETY: reads the calling thread's last-error value immediately.
        return Err(SysError::Os(unsafe { GetLastError() }));
    }
    let handle = ProcessHandle(raw);
    let mut creation = MaybeUninit::<FILETIME>::uninit();
    let mut exit = MaybeUninit::<FILETIME>::uninit();
    let mut kernel = MaybeUninit::<FILETIME>::uninit();
    let mut user = MaybeUninit::<FILETIME>::uninit();
    // SAFETY: the handle is valid and every output points to writable storage.
    let ok = unsafe {
        GetProcessTimes(
            handle.0,
            creation.as_mut_ptr(),
            exit.as_mut_ptr(),
            kernel.as_mut_ptr(),
            user.as_mut_ptr(),
        )
    };
    if ok == 0 {
        // SAFETY: reads last-error immediately after the failing call.
        return Err(SysError::Os(unsafe { GetLastError() }));
    }
    // SAFETY: GetProcessTimes succeeded and initialized every output.
    let creation = unsafe { creation.assume_init() };
    Ok(RawUniqueProcess {
        pid,
        start_time: filetime_to_u64(creation),
    })
}

struct ProcessHandle(HANDLE);

impl Drop for ProcessHandle {
    fn drop(&mut self) {
        // SAFETY: this wrapper owns a non-null handle from OpenProcess.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

fn raw_application(value: &RM_PROCESS_INFO) -> SysResult<RawApplication> {
    Ok(RawApplication {
        display_name: decode_fixed(&value.strAppName)?,
        service_name: decode_fixed(&value.strServiceShortName)?,
        application_type: value.ApplicationType,
        status: value.AppStatus,
        restartable: value.bRestartable != 0,
        process: raw_unique_process(value.Process),
        terminal_session_id: value.TSSessionId,
    })
}

fn raw_unique_process(value: RM_UNIQUE_PROCESS) -> RawUniqueProcess {
    RawUniqueProcess {
        pid: value.dwProcessId,
        start_time: filetime_to_u64(value.ProcessStartTime),
    }
}

fn native_unique_process(value: RawUniqueProcess) -> RM_UNIQUE_PROCESS {
    RM_UNIQUE_PROCESS {
        dwProcessId: value.pid,
        ProcessStartTime: u64_to_filetime(value.start_time),
    }
}

fn filetime_to_u64(value: FILETIME) -> u64 {
    (u64::from(value.dwHighDateTime) << 32) | u64::from(value.dwLowDateTime)
}

fn u64_to_filetime(value: u64) -> FILETIME {
    FILETIME {
        dwLowDateTime: value as u32,
        dwHighDateTime: (value >> 32) as u32,
    }
}

#[derive(Debug)]
struct WideString(Vec<u16>);

impl WideString {
    fn new(value: &OsStr) -> SysResult<Self> {
        let mut encoded = value.encode_wide().collect::<Vec<_>>();
        if encoded.contains(&0) {
            return Err(SysError::InvalidInput(
                "strings may not contain an embedded NUL",
            ));
        }
        encoded.push(0);
        Ok(Self(encoded))
    }

    fn as_ptr(&self) -> *const u16 {
        self.0.as_ptr()
    }
}

fn decode_fixed(value: &[u16]) -> SysResult<OsString> {
    let nul = value
        .iter()
        .position(|unit| *unit == 0)
        .ok_or(SysError::MalformedOutput(
            "a fixed-width UTF-16 field was not NUL-terminated",
        ))?;
    Ok(OsString::from_wide(&value[..nul]))
}

fn absolute_file(path: &Path) -> SysResult<PathBuf> {
    std::path::absolute(path).map_err(|error| {
        error.raw_os_error().map_or(
            SysError::InvalidInput("file path could not be made absolute"),
            |code| SysError::Os(code as u32),
        )
    })
}

fn count(length: usize) -> SysResult<u32> {
    u32::try_from(length).map_err(|_| SysError::CountOverflow)
}

fn pointer_or_null<T>(values: &[T]) -> *const T {
    if values.is_empty() {
        ptr::null()
    } else {
        values.as_ptr()
    }
}

fn check(code: u32) -> SysResult<()> {
    if code == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(SysError::Os(code))
    }
}

fn get_filters(raw: u32) -> SysResult<Vec<RawFilter>> {
    let mut needed = 0;
    // SAFETY: the required-size output is valid and a null buffer is allowed
    // for the initial size query.
    let first = unsafe { RmGetFilterList(raw, ptr::null_mut(), 0, &mut needed) };
    if first == ERROR_SUCCESS && needed == 0 {
        return Ok(Vec::new());
    }
    if first != ERROR_SUCCESS && first != ERROR_MORE_DATA {
        return Err(SysError::Os(first));
    }

    for _ in 0..MAX_LIST_ATTEMPTS {
        if needed == 0 {
            return Ok(Vec::new());
        }
        let mut buffer = AlignedBuffer::new(needed)?;
        let supplied = needed;
        let mut returned_needed = 0;
        // SAFETY: `buffer` is writable for exactly `supplied` bytes and is
        // aligned for RM_FILTER_INFO.
        let code =
            unsafe { RmGetFilterList(raw, buffer.as_mut_ptr(), supplied, &mut returned_needed) };
        if code == ERROR_MORE_DATA {
            needed = returned_needed;
            continue;
        }
        check(code)?;
        let valid = if returned_needed == 0 {
            supplied
        } else {
            returned_needed
        };
        if valid > supplied {
            return Err(SysError::MalformedOutput(
                "RmGetFilterList reported a length larger than its buffer",
            ));
        }
        return parse_filter_buffer(&buffer, valid as usize);
    }

    Err(SysError::DataChanged(ERROR_MORE_DATA))
}

struct AlignedBuffer {
    words: Vec<usize>,
}

impl AlignedBuffer {
    fn new(byte_len: u32) -> SysResult<Self> {
        let byte_len = usize::try_from(byte_len).map_err(|_| SysError::CountOverflow)?;
        let word = mem::size_of::<usize>();
        let words = byte_len
            .checked_add(word - 1)
            .ok_or(SysError::CountOverflow)?
            / word;
        let mut storage = Vec::new();
        storage
            .try_reserve_exact(words)
            .map_err(|_| SysError::CountOverflow)?;
        storage.resize(words, 0);
        Ok(Self { words: storage })
    }

    fn as_ptr(&self) -> *const u8 {
        self.words.as_ptr().cast()
    }

    fn as_mut_ptr(&mut self) -> *mut u8 {
        self.words.as_mut_ptr().cast()
    }
}

fn parse_filter_buffer(buffer: &AlignedBuffer, byte_len: usize) -> SysResult<Vec<RawFilter>> {
    if byte_len == 0 {
        return Ok(Vec::new());
    }
    let record_size = mem::size_of::<RM_FILTER_INFO>();
    if byte_len < record_size {
        return Err(SysError::MalformedOutput(
            "filter buffer is shorter than RM_FILTER_INFO",
        ));
    }
    let base = buffer.as_ptr() as usize;
    let end = base
        .checked_add(byte_len)
        .ok_or(SysError::MalformedOutput("filter buffer range overflowed"))?;
    let mut offset = 0_usize;
    let mut filters = Vec::new();

    loop {
        let record_end = offset
            .checked_add(record_size)
            .ok_or(SysError::MalformedOutput("filter record offset overflowed"))?;
        if record_end > byte_len {
            return Err(SysError::MalformedOutput(
                "filter record lies outside its buffer",
            ));
        }
        // SAFETY: the complete record range was checked. `read_unaligned`
        // avoids imposing alignment requirements on native offsets.
        let record =
            unsafe { ptr::read_unaligned(buffer.as_ptr().add(offset).cast::<RM_FILTER_INFO>()) };
        let target = match record.FilterTrigger {
            RM_FILTER_TRIGGER_FILE => {
                // SAFETY: the active union member is selected by FilterTrigger.
                let pointer = unsafe { record.Anonymous.strFilename };
                RawFilterTarget::Executable(PathBuf::from(read_buffer_string(pointer, base, end)?))
            }
            RM_FILTER_TRIGGER_PROCESS => {
                // SAFETY: the active union member is selected by FilterTrigger.
                let process = unsafe { record.Anonymous.Process };
                RawFilterTarget::Process(raw_unique_process(process))
            }
            RM_FILTER_TRIGGER_SERVICE => {
                // SAFETY: the active union member is selected by FilterTrigger.
                let pointer = unsafe { record.Anonymous.strServiceShortName };
                RawFilterTarget::Service(read_buffer_string(pointer, base, end)?)
            }
            _ => {
                return Err(SysError::MalformedOutput(
                    "filter buffer contains an unknown trigger",
                ));
            }
        };
        filters.push(RawFilter {
            target,
            action: record.FilterAction,
        });

        if record.cbNextOffset == 0 {
            break;
        }
        let next = usize::try_from(record.cbNextOffset).map_err(|_| SysError::CountOverflow)?;
        if next < record_size {
            return Err(SysError::MalformedOutput(
                "filter next offset does not advance past the current record",
            ));
        }
        offset = offset
            .checked_add(next)
            .ok_or(SysError::MalformedOutput("filter next offset overflowed"))?;
    }

    Ok(filters)
}

fn read_buffer_string(pointer: *mut u16, base: usize, end: usize) -> SysResult<OsString> {
    let start = pointer as usize;
    if pointer.is_null() || start < base || start >= end {
        return Err(SysError::MalformedOutput(
            "filter string pointer lies outside its buffer",
        ));
    }
    let mut units = Vec::new();
    let mut cursor = start;
    loop {
        let unit_end =
            cursor
                .checked_add(mem::size_of::<u16>())
                .ok_or(SysError::MalformedOutput(
                    "filter string pointer overflowed",
                ))?;
        if unit_end > end {
            return Err(SysError::MalformedOutput(
                "filter string was not NUL-terminated inside its buffer",
            ));
        }
        // SAFETY: the two-byte range is inside the buffer. Unaligned reads are
        // used because a corrupt OS offset must not become undefined behavior.
        let unit = unsafe { ptr::read_unaligned(cursor as *const u16) };
        if unit == 0 {
            break;
        }
        units.push(unit);
        cursor = unit_end;
    }
    Ok(OsString::from_wide(&units))
}

#[derive(Clone, Copy)]
struct CallbackSlot {
    data: *mut (),
    call: unsafe fn(*mut (), u32),
}

// SAFETY: access to the pointer is serialized by CALLBACK_SLOT, and a lease
// ensures its referent remains alive until the native call and callbacks end.
unsafe impl Send for CallbackSlot {}

static CALLBACK_SLOT: Mutex<Option<CallbackSlot>> = Mutex::new(None);

struct CallbackState<F> {
    callback: *mut F,
    panic: Option<Box<dyn Any + Send>>,
}

impl<F> CallbackState<F> {
    fn new(callback: &mut F) -> Self {
        Self {
            callback,
            panic: None,
        }
    }
}

struct CallbackLease;

impl CallbackLease {
    fn install<F>(state: &mut CallbackState<F>) -> SysResult<Self>
    where
        F: FnMut(u32) + Send,
    {
        let mut slot = match CALLBACK_SLOT.try_lock() {
            Ok(guard) => guard,
            Err(TryLockError::WouldBlock) => return Err(SysError::CallbackInUse),
            Err(TryLockError::Poisoned(error)) => error.into_inner(),
        };
        if slot.is_some() {
            return Err(SysError::CallbackInUse);
        }
        *slot = Some(CallbackSlot {
            data: ptr::from_mut(state).cast(),
            call: call_callback::<F>,
        });
        Ok(Self)
    }
}

impl Drop for CallbackLease {
    fn drop(&mut self) {
        let mut slot = CALLBACK_SLOT
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        *slot = None;
    }
}

unsafe fn call_callback<F>(data: *mut (), percent: u32)
where
    F: FnMut(u32) + Send,
{
    // SAFETY: CallbackLease installed a pointer to CallbackState<F> and keeps
    // that stack value alive until all callbacks have completed.
    let state = unsafe { &mut *data.cast::<CallbackState<F>>() };
    if state.panic.is_some() {
        return;
    }
    // SAFETY: CALLBACK_SLOT serializes FnMut access and the original callback
    // remains borrowed until the lease is removed.
    let callback = unsafe { &mut *state.callback };
    if let Err(payload) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        callback(percent);
    })) {
        state.panic = Some(payload);
    }
}

unsafe extern "system" fn callback_trampoline(percent: u32) {
    // No Rust panic may cross this system ABI boundary. User callback panics
    // are captured in CallbackState and resumed on the initiating Rust thread.
    let _ = std::panic::catch_unwind(|| {
        let slot = CALLBACK_SLOT
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(slot) = *slot {
            // SAFETY: the active lease owns this type-erased call target.
            unsafe { (slot.call)(slot.data, percent) };
        }
    });
}

fn run_with_callback<F, O>(callback: &mut F, operation: O) -> SysResult<()>
where
    F: FnMut(u32) + Send,
    O: FnOnce(RM_WRITE_STATUS_CALLBACK) -> u32,
{
    let mut state = CallbackState::new(callback);
    let lease = CallbackLease::install(&mut state)?;
    let code = operation(Some(callback_trampoline));
    // Clearing the slot takes the mutex and therefore waits for any callback
    // already in progress before the stack state is inspected or dropped.
    drop(lease);
    if let Some(payload) = state.panic.take() {
        std::panic::resume_unwind(payload);
    }
    check(code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::panic::AssertUnwindSafe;

    #[test]
    fn utf16_rejects_nul_and_fixed_fields_require_termination() {
        assert_eq!(
            WideString::new(OsStr::new("a\0b")).unwrap_err(),
            SysError::InvalidInput("strings may not contain an embedded NUL")
        );
        assert!(decode_fixed(&[b'a' as u16, 0, b'b' as u16]).is_ok());
        assert!(matches!(
            decode_fixed(&[b'a' as u16]),
            Err(SysError::MalformedOutput(_))
        ));
    }

    #[test]
    fn filetime_round_trip_preserves_all_bits() {
        let value = 0xFEDC_BA98_7654_3210;
        assert_eq!(filetime_to_u64(u64_to_filetime(value)), value);
    }

    #[test]
    fn filter_parser_validates_offsets_pointers_and_termination() {
        let text = OsStr::new("demo.exe")
            .encode_wide()
            .chain([0])
            .collect::<Vec<_>>();
        let record_size = mem::size_of::<RM_FILTER_INFO>();
        let byte_len = record_size + text.len() * mem::size_of::<u16>();
        let mut buffer = AlignedBuffer::new(byte_len as u32).unwrap();
        let text_pointer = unsafe { buffer.as_mut_ptr().add(record_size).cast::<u16>() };
        unsafe {
            ptr::copy_nonoverlapping(text.as_ptr(), text_pointer, text.len());
        }
        let mut target = windows_sys::Win32::System::RestartManager::RM_FILTER_INFO_0::default();
        target.strFilename = text_pointer;
        let record = RM_FILTER_INFO {
            FilterAction: RmNoRestart,
            FilterTrigger: RM_FILTER_TRIGGER_FILE,
            cbNextOffset: 0,
            Anonymous: target,
        };
        unsafe {
            ptr::write_unaligned(buffer.as_mut_ptr().cast::<RM_FILTER_INFO>(), record);
        }

        let parsed = parse_filter_buffer(&buffer, byte_len).unwrap();
        assert_eq!(
            parsed,
            vec![RawFilter {
                target: RawFilterTarget::Executable(PathBuf::from("demo.exe")),
                action: RmNoRestart,
            }]
        );

        let mut invalid = record;
        invalid.Anonymous.strFilename = (buffer.as_ptr() as usize + byte_len + 2) as *mut u16;
        unsafe {
            ptr::write_unaligned(buffer.as_mut_ptr().cast::<RM_FILTER_INFO>(), invalid);
        }
        assert!(matches!(
            parse_filter_buffer(&buffer, byte_len),
            Err(SysError::MalformedOutput(_))
        ));

        invalid.cbNextOffset = 1;
        invalid.Anonymous.strFilename = text_pointer;
        unsafe {
            ptr::write_unaligned(buffer.as_mut_ptr().cast::<RM_FILTER_INFO>(), invalid);
        }
        assert!(matches!(
            parse_filter_buffer(&buffer, byte_len),
            Err(SysError::MalformedOutput(_))
        ));
    }

    #[test]
    fn callback_lease_is_exclusive_and_resumes_panics_after_release() {
        let mut first = |_| {};
        let mut first_state = CallbackState::new(&mut first);
        let lease = CallbackLease::install(&mut first_state).unwrap();
        let mut second = |_| {};
        assert_eq!(
            run_with_callback(&mut second, |_| ERROR_SUCCESS).unwrap_err(),
            SysError::CallbackInUse
        );
        drop(lease);

        let panic = std::panic::catch_unwind(AssertUnwindSafe(|| {
            let mut callback = |_| panic!("callback panic");
            run_with_callback(&mut callback, |native| {
                unsafe { native.unwrap()(50) };
                ERROR_SUCCESS
            })
            .unwrap();
        }));
        assert!(panic.is_err());

        let mut called = false;
        let mut callback = |_| called = true;
        run_with_callback(&mut callback, |native| {
            unsafe { native.unwrap()(100) };
            ERROR_SUCCESS
        })
        .unwrap();
        assert!(called);
    }

    #[test]
    fn changing_lists_have_a_bounded_retry_budget() {
        assert_eq!(MAX_LIST_ATTEMPTS, 8);
        let attempts = (0..MAX_LIST_ATTEMPTS).count();
        assert_eq!(attempts, 8);
        assert_eq!(
            SysError::DataChanged(ERROR_MORE_DATA),
            SysError::DataChanged(234)
        );
    }
}
