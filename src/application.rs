//! Domain types describing applications affected by registered resources.

use std::ffi::{OsStr, OsString};

use bitflags::bitflags;

/// A process identity made from its ID and creation time.
///
/// Pairing both values prevents a recycled PID from identifying an unrelated
/// newer process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProcessIdentity {
    pid: u32,
    creation_time_100ns_since_1601: u64,
}

impl ProcessIdentity {
    /// Builds an identity from a process ID and a Windows `FILETIME` value.
    ///
    /// The creation time is counted in 100-nanosecond units since
    /// 1601-01-01 UTC. PID 0 and the native invalid sentinel are rejected.
    pub fn from_raw_parts(pid: u32, creation_time_100ns_since_1601: u64) -> crate::Result<Self> {
        if pid == 0 || pid == u32::MAX {
            return Err(crate::Error::new(
                crate::ErrorKind::InvalidInput,
                None,
                "a process identity PID must be neither zero nor the native invalid sentinel",
            ));
        }
        Ok(Self {
            pid,
            creation_time_100ns_since_1601,
        })
    }

    pub(crate) const fn from_raw_parts_unchecked(
        pid: u32,
        creation_time_100ns_since_1601: u64,
    ) -> Self {
        Self {
            pid,
            creation_time_100ns_since_1601,
        }
    }

    /// Returns the process identifier.
    #[must_use]
    pub const fn pid(self) -> u32 {
        self.pid
    }

    /// Returns the creation time in 100-nanosecond ticks since 1601-01-01 UTC.
    #[must_use]
    pub const fn creation_time_100ns_since_1601(self) -> u64 {
        self.creation_time_100ns_since_1601
    }
}

/// The kind of an affected application, corresponding to `RM_APP_TYPE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ApplicationType {
    /// Windows explicitly reported `RmUnknownApp`.
    Unknown,
    /// An application with a top-level window.
    MainWindow,
    /// An application without a top-level window.
    OtherWindow,
    /// A Windows service.
    Service,
    /// Windows Explorer.
    Explorer,
    /// A console application.
    Console,
    /// Windows reported `RmCritical`.
    ///
    /// This can describe applications that cannot be shut down without a
    /// reboot; it is not merely a process criticality flag.
    Critical,
    /// A value introduced by a newer Windows version.
    Unrecognized(i32),
}

impl ApplicationType {
    pub(crate) const fn from_raw(value: i32) -> Self {
        match value {
            0 => Self::Unknown,
            1 => Self::MainWindow,
            2 => Self::OtherWindow,
            3 => Self::Service,
            4 => Self::Explorer,
            5 => Self::Console,
            1000 => Self::Critical,
            other => Self::Unrecognized(other),
        }
    }

    /// Returns the underlying `RM_APP_TYPE` integer.
    #[must_use]
    pub const fn raw_value(self) -> i32 {
        match self {
            Self::Unknown => 0,
            Self::MainWindow => 1,
            Self::OtherWindow => 2,
            Self::Service => 3,
            Self::Explorer => 4,
            Self::Console => 5,
            Self::Critical => 1000,
            Self::Unrecognized(value) => value,
        }
    }
}

bitflags! {
    /// OR-able history bits from `RM_APP_STATUS`.
    ///
    /// Zero means that Windows did not report a known state. Unknown bits are
    /// retained so newer Windows releases remain lossless.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct ApplicationStatus: u32 {
        /// The application is running.
        const RUNNING = 0x01;
        /// Restart Manager stopped the application.
        const STOPPED = 0x02;
        /// Something other than Restart Manager stopped the application.
        const STOPPED_OTHER = 0x04;
        /// Restart Manager restarted the application.
        const RESTARTED = 0x08;
        /// Restart Manager could not stop the application.
        const ERROR_ON_STOP = 0x10;
        /// Restart Manager could not restart the application.
        const ERROR_ON_RESTART = 0x20;
        /// A filter masked shutdown.
        const SHUTDOWN_MASKED = 0x40;
        /// A filter masked restart.
        const RESTART_MASKED = 0x80;
    }
}

bitflags! {
    /// Reasons Windows says a reboot may be required.
    ///
    /// Unknown bits are retained.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct RebootReasons: u32 {
        /// Access was denied while inspecting or acting on a resource.
        const PERMISSION_DENIED = 0x01;
        /// An affected process is in a different Terminal Services session.
        const SESSION_MISMATCH = 0x02;
        /// An affected process was reported as critical.
        const CRITICAL_PROCESS = 0x04;
        /// An affected service was reported as critical.
        const CRITICAL_SERVICE = 0x08;
        /// Restart Manager detected the caller among the affected processes.
        const DETECTED_SELF = 0x10;
    }
}

/// One application or service using a registered resource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AffectedApplication {
    pub(crate) display_name: OsString,
    pub(crate) service_name: Option<OsString>,
    pub(crate) application_type: ApplicationType,
    pub(crate) status: ApplicationStatus,
    pub(crate) restartable: bool,
    pub(crate) process: Option<ProcessIdentity>,
    pub(crate) terminal_session_id: Option<u32>,
}

impl AffectedApplication {
    /// Returns the display name without lossy Unicode conversion.
    #[must_use]
    pub fn display_name(&self) -> &OsStr {
        &self.display_name
    }

    /// Returns the service short name for service entries.
    #[must_use]
    pub fn service_name(&self) -> Option<&OsStr> {
        self.service_name.as_deref()
    }

    /// Returns the kind reported by Windows.
    #[must_use]
    pub const fn application_type(&self) -> ApplicationType {
        self.application_type
    }

    /// Returns the accumulated status/history bits.
    #[must_use]
    pub const fn status(&self) -> ApplicationStatus {
        self.status
    }

    /// Reports whether Windows can restart this application.
    #[must_use]
    pub const fn is_restartable(&self) -> bool {
        self.restartable
    }

    /// Returns the process identity, if this entry has a valid process.
    #[must_use]
    pub const fn process(&self) -> Option<ProcessIdentity> {
        self.process
    }

    /// Returns the Terminal Services session ID when Windows supplied one.
    #[must_use]
    pub const fn terminal_session_id(&self) -> Option<u32> {
        self.terminal_session_id
    }
}

/// A reusable affected-application report plus its reboot reasons.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AffectedApplications {
    pub(crate) applications: Vec<AffectedApplication>,
    pub(crate) reboot_reasons: RebootReasons,
}

impl AffectedApplications {
    /// Returns the affected entries.
    #[must_use]
    pub fn applications(&self) -> &[AffectedApplication] {
        &self.applications
    }

    /// Returns the reboot reasons supplied with this snapshot.
    #[must_use]
    pub const fn reboot_reasons(&self) -> RebootReasons {
        self.reboot_reasons
    }

    /// Returns an iterator over the affected entries.
    pub fn iter(&self) -> std::slice::Iter<'_, AffectedApplication> {
        self.applications.iter()
    }

    /// Returns whether the report contains no affected entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.applications.is_empty()
    }

    /// Returns the number of affected entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.applications.len()
    }
}

impl IntoIterator for AffectedApplications {
    type Item = AffectedApplication;
    type IntoIter = std::vec::IntoIter<AffectedApplication>;

    fn into_iter(self) -> Self::IntoIter {
        self.applications.into_iter()
    }
}

impl<'a> IntoIterator for &'a AffectedApplications {
    type Item = &'a AffectedApplication;
    type IntoIter = std::slice::Iter<'a, AffectedApplication>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn application_type_distinguishes_unknown_and_future_values() {
        let values = [0, 1, 2, 3, 4, 5, 1000, 77];
        for value in values {
            assert_eq!(ApplicationType::from_raw(value).raw_value(), value);
        }
        assert_eq!(ApplicationType::from_raw(0), ApplicationType::Unknown);
        assert_eq!(
            ApplicationType::from_raw(77),
            ApplicationType::Unrecognized(77)
        );
    }

    #[test]
    fn status_and_reboot_reasons_retain_unknown_bits() {
        let status = ApplicationStatus::from_bits_retain(0x4000_0001);
        assert!(status.contains(ApplicationStatus::RUNNING));
        assert_eq!(status.bits(), 0x4000_0001);

        let reasons = RebootReasons::from_bits_retain(0x8000_0002);
        assert!(reasons.contains(RebootReasons::SESSION_MISMATCH));
        assert_eq!(reasons.bits(), 0x8000_0002);
    }

    #[test]
    fn affected_report_accessors_and_iterators_are_reusable() {
        let process = ProcessIdentity::from_raw_parts(42, 99).unwrap();
        assert_eq!(process.pid(), 42);
        assert_eq!(process.creation_time_100ns_since_1601(), 99);
        let application = AffectedApplication {
            display_name: OsString::from("display"),
            service_name: Some(OsString::from("service")),
            application_type: ApplicationType::Service,
            status: ApplicationStatus::RUNNING | ApplicationStatus::RESTARTED,
            restartable: true,
            process: Some(process),
            terminal_session_id: Some(7),
        };
        assert_eq!(application.display_name(), OsStr::new("display"));
        assert_eq!(application.service_name(), Some(OsStr::new("service")));
        assert_eq!(application.application_type(), ApplicationType::Service);
        assert!(application.status().contains(ApplicationStatus::RUNNING));
        assert!(application.is_restartable());
        assert_eq!(application.process(), Some(process));
        assert_eq!(application.terminal_session_id(), Some(7));

        let report = AffectedApplications {
            applications: vec![application],
            reboot_reasons: RebootReasons::DETECTED_SELF,
        };
        assert_eq!(report.len(), 1);
        assert!(!report.is_empty());
        assert_eq!(report.reboot_reasons(), RebootReasons::DETECTED_SELF);
        assert_eq!(report.iter().count(), 1);
        assert_eq!((&report).into_iter().count(), 1);
        assert_eq!(report.into_iter().count(), 1);
    }

    #[test]
    fn process_identity_rejects_native_invalid_pids() {
        assert!(ProcessIdentity::from_raw_parts(0, 1).is_err());
        assert!(ProcessIdentity::from_raw_parts(u32::MAX, 1).is_err());
        assert!(ProcessIdentity::from_raw_parts(1, 1).is_ok());
    }
}
