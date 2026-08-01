//! Stable error classification without exposing implementation details.

/// Specialised result alias used by this crate.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// A stable, non-exhaustive classification of failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorKind {
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
    /// A collection was too large for the native `u32` count fields.
    TooManyResources,
    /// Another process-wide progress callback operation is active.
    CallbackInUse,
    /// The affected-application or filter list changed through every retry.
    DataChanged,
    /// Windows returned an internally inconsistent buffer.
    MalformedOsData,
    /// The weak cancellation capability refers to an ended session.
    SessionEnded,
    /// Another Windows error not covered by a more specific classification.
    Os,
}

/// An error returned by the safe Restart Manager API.
///
/// Its representation is private. Match on [`Error::kind`] and use
/// [`Error::raw_os_error`] when the precise Win32 value matters.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct Error {
    kind: ErrorKind,
    raw_os_error: Option<u32>,
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

    pub(crate) fn new(
        kind: ErrorKind,
        raw_os_error: Option<u32>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            raw_os_error,
            message: message.into(),
        }
    }
}
