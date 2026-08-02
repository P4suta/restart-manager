//! Process-global application restart registration.

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::input::contains_nul;
use crate::session::map_sys_error;
use crate::{Error, ErrorKind, Result};

const MAX_RESTART_ARGUMENT_UNITS: usize = 1024;
static REGISTRATION_OWNED: AtomicBool = AtomicBool::new(false);

/// Command-line arguments and positive restart policies for this application.
///
/// The executable name must not be included. The default permits restart
/// after every supported reason.
#[derive(Clone, PartialEq, Eq)]
pub struct ApplicationRestartOptions {
    arguments: OsString,
    restart_on_crash: bool,
    restart_on_hang: bool,
    restart_on_update: bool,
    restart_on_reboot: bool,
}

impl Default for ApplicationRestartOptions {
    fn default() -> Self {
        Self {
            arguments: OsString::new(),
            restart_on_crash: true,
            restart_on_hang: true,
            restart_on_update: true,
            restart_on_reboot: true,
        }
    }
}

impl fmt::Debug for ApplicationRestartOptions {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ApplicationRestartOptions")
            .field("arguments", &"<redacted>")
            .field("restart_on_crash", &self.restart_on_crash)
            .field("restart_on_hang", &self.restart_on_hang)
            .field("restart_on_update", &self.restart_on_update)
            .field("restart_on_reboot", &self.restart_on_reboot)
            .finish()
    }
}

impl ApplicationRestartOptions {
    /// Creates options with the supplied restart arguments and all reasons enabled.
    #[must_use]
    pub fn new(arguments: impl Into<OsString>) -> Self {
        Self {
            arguments: arguments.into(),
            ..Self::default()
        }
    }

    /// Replaces the restart arguments.
    #[must_use]
    pub fn with_arguments(mut self, arguments: impl Into<OsString>) -> Self {
        self.arguments = arguments.into();
        self
    }

    /// Chooses whether restart is allowed after an unhandled crash.
    #[must_use]
    pub const fn with_restart_on_crash(mut self, enabled: bool) -> Self {
        self.restart_on_crash = enabled;
        self
    }

    /// Chooses whether restart is allowed after an application hang.
    #[must_use]
    pub const fn with_restart_on_hang(mut self, enabled: bool) -> Self {
        self.restart_on_hang = enabled;
        self
    }

    /// Chooses whether restart is allowed after an update.
    #[must_use]
    pub const fn with_restart_on_update(mut self, enabled: bool) -> Self {
        self.restart_on_update = enabled;
        self
    }

    /// Chooses whether restart is allowed after a system reboot.
    #[must_use]
    pub const fn with_restart_on_reboot(mut self, enabled: bool) -> Self {
        self.restart_on_reboot = enabled;
        self
    }

    /// Returns whether crash restart is allowed.
    #[must_use]
    pub const fn restart_on_crash(&self) -> bool {
        self.restart_on_crash
    }

    /// Returns whether hang restart is allowed.
    #[must_use]
    pub const fn restart_on_hang(&self) -> bool {
        self.restart_on_hang
    }

    /// Returns whether update restart is allowed.
    #[must_use]
    pub const fn restart_on_update(&self) -> bool {
        self.restart_on_update
    }

    /// Returns whether reboot restart is allowed.
    #[must_use]
    pub const fn restart_on_reboot(&self) -> bool {
        self.restart_on_reboot
    }

    fn native_flags(&self) -> u32 {
        ((!self.restart_on_crash) as u32)
            | (((!self.restart_on_hang) as u32) << 1)
            | (((!self.restart_on_update) as u32) << 2)
            | (((!self.restart_on_reboot) as u32) << 3)
    }
}

/// An exclusive crate-managed lease for the current process registration.
pub struct ApplicationRestartRegistration {
    options: ApplicationRestartOptions,
    armed: bool,
}

impl fmt::Debug for ApplicationRestartRegistration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ApplicationRestartRegistration")
            .field("options", &self.options)
            .field("armed", &self.armed)
            .finish()
    }
}

impl ApplicationRestartRegistration {
    /// Registers this process for application restart.
    pub fn register(options: ApplicationRestartOptions) -> Result<Self> {
        validate_arguments(&options.arguments)?;
        if REGISTRATION_OWNED
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(Error::new(
                ErrorKind::ApplicationRestartInUse,
                None,
                "this process already owns an application restart registration",
            ));
        }
        if let Err(error) =
            crate::sys::register_application_restart(&options.arguments, options.native_flags())
        {
            REGISTRATION_OWNED.store(false, Ordering::Release);
            return Err(map_sys_error(error));
        }
        Ok(Self {
            options,
            armed: true,
        })
    }

    /// Updates the command line and policies without releasing the lease.
    pub fn update(&mut self, options: ApplicationRestartOptions) -> Result<()> {
        validate_arguments(&options.arguments)?;
        crate::sys::register_application_restart(&options.arguments, options.native_flags())
            .map_err(map_sys_error)?;
        self.options = options;
        Ok(())
    }

    /// Explicitly unregisters this process.
    ///
    /// On failure the value remains armed and its destructor makes one
    /// best-effort retry.
    pub fn unregister(mut self) -> Result<()> {
        match crate::sys::unregister_application_restart() {
            Ok(()) => {
                self.disarm();
                Ok(())
            }
            Err(error) => Err(map_sys_error(error)),
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
        REGISTRATION_OWNED.store(false, Ordering::Release);
    }
}

impl Drop for ApplicationRestartRegistration {
    fn drop(&mut self) {
        if self.armed {
            let _ = crate::sys::unregister_application_restart();
            self.disarm();
        }
    }
}

fn validate_arguments(arguments: &OsStr) -> Result<()> {
    if contains_nul(arguments) {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            None,
            "application restart arguments may not contain an embedded NUL",
        ));
    }
    if utf16_unit_count(arguments) > MAX_RESTART_ARGUMENT_UNITS {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            None,
            "application restart arguments exceed 1024 UTF-16 code units",
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn utf16_unit_count(value: &OsStr) -> usize {
    use std::os::windows::ffi::OsStrExt;
    value.encode_wide().count()
}

#[cfg(not(windows))]
fn utf16_unit_count(value: &OsStr) -> usize {
    value.to_string_lossy().encode_utf16().count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_use_positive_policies_and_redact_arguments() {
        let options = ApplicationRestartOptions::new("secret")
            .with_restart_on_crash(false)
            .with_restart_on_reboot(false);
        assert!(!options.restart_on_crash());
        assert!(options.restart_on_hang());
        assert!(options.restart_on_update());
        assert!(!options.restart_on_reboot());
        assert_eq!(options.native_flags(), 0x09);
        let debug = format!("{options:?}");
        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains("secret"));
    }

    #[test]
    fn arguments_validate_nul_and_utf16_limit() {
        assert!(validate_arguments(OsStr::new("bad\0argument")).is_err());
        assert!(validate_arguments(OsStr::new(&"x".repeat(1024))).is_ok());
        assert!(validate_arguments(OsStr::new(&"x".repeat(1025))).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn all_builders_registration_debug_and_drop_are_exercised() {
        let options = ApplicationRestartOptions::default()
            .with_arguments("--updated")
            .with_restart_on_hang(false)
            .with_restart_on_update(false)
            .with_restart_on_reboot(false);
        assert!(options.restart_on_crash());
        assert!(!options.restart_on_hang());
        assert!(!options.restart_on_update());
        assert!(!options.restart_on_reboot());

        let registration = ApplicationRestartRegistration::register(options).unwrap();
        let debug = format!("{registration:?}");
        assert!(debug.contains("ApplicationRestartRegistration"));
        assert!(debug.contains("<redacted>"));
        drop(registration);

        ApplicationRestartRegistration::register(ApplicationRestartOptions::default())
            .unwrap()
            .unregister()
            .unwrap();
    }
}
