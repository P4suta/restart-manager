//! Stable error classification without exposing implementation details.

/// Specialised result alias used by this crate.
pub type Result<T> = std::result::Result<T, Error>;

/// A stable, non-exhaustive classification of failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorKind {
    /// The requested operating-system operation is unavailable on this target.
    UnsupportedPlatform,
    /// All 64 Restart Manager session slots in the user session are occupied.
    SessionLimit,
    /// A session key was malformed, expired, or otherwise rejected.
    InvalidSessionKey,
    /// A shutdown or restart operation was cancelled.
    Cancelled,
    /// Windows denied access to a resource or process.
    AccessDenied,
    /// Input could not be represented by the native API.
    InvalidInput,
    /// Restart Manager does not accept directories as file resources.
    DirectoryNotSupported,
    /// A collection was too large for the native `u32` count fields.
    TooManyResources,
    /// Another process-wide progress callback operation is active.
    CallbackInUse,
    /// The affected-application or filter list changed through every retry.
    DataChanged,
    /// Windows returned an internally inconsistent buffer.
    MalformedOsData,
    /// Windows requires a system reboot before this operation can proceed.
    RebootRequired,
    /// At least one affected application could not be shut down.
    ShutdownIncomplete,
    /// At least one stopped application could not be restarted.
    RestartIncomplete,
    /// An operation was requested in an invalid native sequence.
    OperationOutOfSequence,
    /// Restart Manager could not access its registry state.
    RegistryUnavailable,
    /// The selected filter does not exist.
    FilterNotFound,
    /// The operating system could not allocate required memory.
    OutOfMemory,
    /// This process already owns a crate-managed restart registration.
    ApplicationRestartInUse,
    /// A dedicated asynchronous worker thread could not be created or failed.
    AsyncWorkerUnavailable,
    /// The weak cancellation capability refers to an ended session.
    SessionEnded,
    /// Another Windows error not covered by a more specific classification.
    Os,
}

/// An error returned by the safe Restart Manager API.
///
/// Its representation is private. Match on [`Error::kind`] and use
/// [`Error::raw_os_error`] when the precise Win32 value matters.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct Error {
    kind: ErrorKind,
    raw_os_error: Option<u32>,
    raw_hresult: Option<i32>,
    message: String,
}

impl Error {
    /// Returns the stable classification for this error.
    #[must_use]
    pub const fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// Returns the original Win32 error code, when the failure came from Windows.
    #[must_use]
    pub const fn raw_os_error(&self) -> Option<u32> {
        self.raw_os_error
    }

    /// Returns the original HRESULT for application-restart failures.
    #[must_use]
    pub const fn raw_hresult(&self) -> Option<i32> {
        self.raw_hresult
    }

    pub(crate) fn new(
        kind: ErrorKind,
        raw_os_error: Option<u32>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            raw_os_error,
            raw_hresult: None,
            message: message.into(),
        }
    }

    pub(crate) fn from_hresult(kind: ErrorKind, hresult: i32, message: impl Into<String>) -> Self {
        Self {
            kind,
            raw_os_error: None,
            raw_hresult: Some(hresult),
            message: message.into(),
        }
    }
}

/// Error returned when parsing a Restart Manager session key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[error("a session key must contain exactly 32 ASCII hexadecimal characters")]
pub struct ParseSessionKeyError;
