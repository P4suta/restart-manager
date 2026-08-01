//! Options and progress values for shutdown and restart operations.

/// Options accepted by [`crate::RestartSession::shutdown_with_options`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ShutdownOptions {
    force_if_hung: bool,
    only_registered: bool,
}

impl ShutdownOptions {
    /// Creates the graceful default: never force and allow non-restartable apps.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            force_if_hung: false,
            only_registered: false,
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

    /// Chooses whether shutdown proceeds only when every affected application
    /// has registered for restart.
    #[must_use]
    pub const fn with_only_registered(mut self, enabled: bool) -> Self {
        self.only_registered = enabled;
        self
    }

    /// Returns whether forced termination is enabled.
    #[must_use]
    pub const fn force_if_hung(self) -> bool {
        self.force_if_hung
    }

    /// Returns whether all affected applications must be restartable.
    #[must_use]
    pub const fn only_registered(self) -> bool {
        self.only_registered
    }

    pub(crate) const fn native_flags(self) -> u32 {
        let mut flags = 0;
        if self.force_if_hung {
            flags |= 0x01;
        }
        if self.only_registered {
            flags |= 0x10;
        }
        flags
    }
}

/// Progress reported by a blocking shutdown or restart operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Progress {
    percent_complete: u32,
}

impl Progress {
    pub(crate) const fn from_native(percent_complete: u32) -> Self {
        Self {
            percent_complete: if percent_complete > 100 {
                100
            } else {
                percent_complete
            },
        }
    }

    /// Returns a value in `0..=100`.
    #[must_use]
    pub const fn percent_complete(self) -> u32 {
        self.percent_complete
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_is_bounded() {
        assert_eq!(Progress::from_native(42).percent_complete(), 42);
        assert_eq!(Progress::from_native(101).percent_complete(), 100);
    }

    #[test]
    fn shutdown_flags_are_composable() {
        let defaults = ShutdownOptions::default();
        assert!(!defaults.force_if_hung());
        assert!(!defaults.only_registered());
        let options = ShutdownOptions::new()
            .with_force_if_hung(true)
            .with_only_registered(true);
        assert_eq!(options.native_flags(), 0x11);
        assert!(options.force_if_hung());
        assert!(options.only_registered());
    }
}
