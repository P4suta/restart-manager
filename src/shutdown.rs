//! Options and outcomes for shutdown and restart operations.

use crate::{Error, Result};

/// Options accepted by [`crate::RestartSession::shutdown_with_options`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ShutdownOptions {
    force_if_hung: bool,
    require_restart_registration: bool,
}

impl ShutdownOptions {
    /// Creates the graceful default.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            force_if_hung: false,
            require_restart_registration: false,
        }
    }

    /// Chooses whether unresponsive applications may be force-terminated.
    ///
    /// Enabling this can cause data loss in affected applications.
    #[must_use]
    pub const fn with_force_if_hung(mut self, enabled: bool) -> Self {
        self.force_if_hung = enabled;
        self
    }

    /// Requires every affected application to have registered for restart.
    #[must_use]
    pub const fn with_require_restart_registration(mut self, enabled: bool) -> Self {
        self.require_restart_registration = enabled;
        self
    }

    /// Returns whether forced termination is enabled.
    #[must_use]
    pub const fn force_if_hung(self) -> bool {
        self.force_if_hung
    }

    /// Returns whether every affected application must be restartable.
    #[must_use]
    pub const fn require_restart_registration(self) -> bool {
        self.require_restart_registration
    }

    pub(crate) const fn native_flags(self) -> u32 {
        let mut flags = 0;
        if self.force_if_hung {
            flags |= 0x01;
        }
        if self.require_restart_registration {
            flags |= 0x10;
        }
        flags
    }
}

/// The retained result of one native shutdown or restart attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperationOutcome {
    /// The native operation completed successfully.
    Succeeded,
    /// The native operation returned an error.
    Failed(Error),
}

impl OperationOutcome {
    pub(crate) fn from_result(result: Result<()>) -> Self {
        match result {
            Ok(()) => Self::Succeeded,
            Err(error) => Self::Failed(error),
        }
    }

    /// Returns whether the operation succeeded.
    #[must_use]
    pub const fn is_success(&self) -> bool {
        matches!(self, Self::Succeeded)
    }

    /// Returns the retained error, if any.
    #[must_use]
    pub const fn error(&self) -> Option<&Error> {
        match self {
            Self::Succeeded => None,
            Self::Failed(error) => Some(error),
        }
    }

    /// Converts the retained value back into the crate result type.
    pub fn into_result(self) -> Result<()> {
        match self {
            Self::Succeeded => Ok(()),
            Self::Failed(error) => Err(error),
        }
    }
}

/// Results from the shutdown attempt and the optional restart attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryOutcome {
    pub(crate) shutdown: OperationOutcome,
    pub(crate) restart: Option<OperationOutcome>,
}

impl RecoveryOutcome {
    /// Returns the shutdown result.
    #[must_use]
    pub const fn shutdown_outcome(&self) -> &OperationOutcome {
        &self.shutdown
    }

    /// Returns the restart result, or `None` after `leave_stopped`.
    #[must_use]
    pub const fn restart_outcome(&self) -> Option<&OperationOutcome> {
        self.restart.as_ref()
    }
}

/// Progress reported by a blocking shutdown or restart operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Progress {
    percent_complete: u8,
}

impl Progress {
    pub(crate) fn try_from_native(percent_complete: u32) -> Option<Self> {
        u8::try_from(percent_complete)
            .ok()
            .filter(|percent| *percent <= 100)
            .map(|percent_complete| Self { percent_complete })
    }

    /// Returns a validated value in `0..=100`.
    #[must_use]
    pub const fn percent_complete(self) -> u8 {
        self.percent_complete
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_is_validated_and_ordered() {
        assert_eq!(
            Progress::try_from_native(42).unwrap().percent_complete(),
            42
        );
        assert!(Progress::try_from_native(101).is_none());
        assert!(Progress::try_from_native(10) < Progress::try_from_native(20));
    }

    #[test]
    fn shutdown_flags_are_composable() {
        let options = ShutdownOptions::new()
            .with_force_if_hung(true)
            .with_require_restart_registration(true);
        assert_eq!(options.native_flags(), 0x11);
        assert!(options.force_if_hung());
        assert!(options.require_restart_registration());
    }

    #[test]
    fn operation_outcomes_preserve_success_and_failure() {
        let success = OperationOutcome::from_result(Ok(()));
        assert!(success.is_success());
        assert!(success.error().is_none());
        success.into_result().unwrap();

        let error = Error::new(
            crate::ErrorKind::Cancelled,
            Some(1223),
            "cancelled for test",
        );
        let failure = OperationOutcome::from_result(Err(error.clone()));
        assert!(!failure.is_success());
        assert_eq!(failure.error(), Some(&error));
        assert_eq!(failure.into_result().unwrap_err(), error);
    }
}
