//! Error type: thin, `thiserror`-derived, always carrying the Win32 code.

/// Specialised `Result` alias used throughout this crate.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Errors reported by the Restart Manager.
///
/// Every variant keeps the raw Win32 error code, so callers that need the
/// exact OS-level cause never lose information to the classification.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// `ERROR_MAX_SESSS_REACHED`: Windows allows at most **64 concurrent
    /// Restart Manager sessions per user session**, and they are all in use.
    ///
    /// Sessions leaked by other (non-RAII) code count against the same
    /// budget, which is exactly why [`crate::RestartSession`] releases its
    /// session unconditionally on `Drop`.
    #[error(
        "the per-user-session limit of 64 concurrent Restart Manager sessions \
         has been reached (Win32 error {code})"
    )]
    SessionLimit {
        /// Raw Win32 error code.
        code: u32,
    },

    /// The session key is malformed, or refers to a session that no longer
    /// exists (relevant to [`crate::RestartSession::join`]).
    #[error("invalid or expired Restart Manager session key (Win32 error {code})")]
    InvalidSessionKey {
        /// Raw Win32 error code.
        code: u32,
    },

    /// `ERROR_CANCELLED`: the current shutdown/restart was cancelled, e.g.
    /// via [`crate::RestartSession::cancel`].
    #[error("the Restart Manager operation was cancelled (Win32 error {code})")]
    Cancelled {
        /// Raw Win32 error code.
        code: u32,
    },

    /// `ERROR_ACCESS_DENIED`: the caller lacks the rights to act on a
    /// registered resource (typically a service, or another user's process).
    #[error("access denied by the Restart Manager (Win32 error {code})")]
    AccessDenied {
        /// Raw Win32 error code.
        code: u32,
    },

    /// Any other Win32 failure returned by a `Rm*` function.
    #[error("Restart Manager call failed (Win32 error {code})")]
    Os {
        /// Raw Win32 error code.
        code: u32,
    },
}

impl Error {
    /// Returns the raw Win32 error code, regardless of the variant.
    pub fn code(&self) -> u32 {
        todo!()
    }
}
