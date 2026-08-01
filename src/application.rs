//! What the Restart Manager found: applications affected by your resources.

use crate::error::Result;
use crate::session::RestartSession;

/// A process identified the way `RM_UNIQUE_PROCESS` identifies one: process
/// ID **plus** process start time.
///
/// PIDs are recycled aggressively on Windows; pairing the PID with the
/// creation time guarantees a stale entry can never point at an innocent
/// newcomer that happens to reuse the number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UniqueProcess {
    /// Process identifier.
    pub pid: u32,
    /// Process creation time as a Windows `FILETIME` tick count
    /// (100-nanosecond intervals since 1601-01-01 UTC).
    pub start_time: u64,
}

/// The kind of an affected application, mirroring `RM_APP_TYPE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ApplicationType {
    /// Application with a top-level window (`RmMainWindow`).
    MainWindow,
    /// Application without a top-level window (`RmOtherWindow`).
    OtherWindow,
    /// A Windows service (`RmService`).
    Service,
    /// Windows Explorer (`RmExplorer`).
    Explorer,
    /// Console application (`RmConsole`).
    Console,
    /// Critical system process; shutting it down requires a reboot
    /// (`RmCritical`).
    Critical,
}

/// Current status of an affected application, mirroring `RM_APP_STATUS`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ApplicationStatus {
    /// The application state is not known (`RmStatusUnknown`).
    Unknown,
    /// The application is running (`RmStatusRunning`).
    Running,
    /// Stopped by the Restart Manager (`RmStatusStopped`).
    Stopped,
    /// Stopped by other means (`RmStatusStoppedOther`).
    StoppedOther,
    /// Restarted by the Restart Manager (`RmStatusRestarted`).
    Restarted,
    /// The Restart Manager failed to stop it (`RmStatusErrorOnStop`).
    ErrorOnStop,
    /// The Restart Manager failed to restart it (`RmStatusErrorOnRestart`).
    ErrorOnRestart,
    /// Excluded from shutdown by a filter (`RmStatusShutdownMasked`).
    ShutdownMasked,
}

/// One application that currently uses at least one registered resource.
#[derive(Debug, Clone)]
pub struct AffectedApplication {
    /// Human-readable name: window title, service display name, or
    /// executable name, depending on [`app_type`](Self::app_type).
    pub display_name: String,
    /// What kind of application this is.
    pub app_type: ApplicationType,
    /// Its current lifecycle status within this session.
    pub status: ApplicationStatus,
    /// Whether the application supports being brought back by
    /// [`RestartSession::restart`] (e.g. it called
    /// `RegisterApplicationRestart`, or it is a service).
    pub restartable: bool,
    /// The concrete process, when one is associated with the entry.
    pub process: Option<UniqueProcess>,
}

impl RestartSession {
    /// Lists the applications and services that hold any of the registered
    /// resources (`RmGetList`).
    ///
    /// `RmGetList` requires the classic `ERROR_MORE_DATA` two-phase call —
    /// ask for the required buffer size, then fetch into it, retrying if the
    /// set changed in between. This method performs that dance internally;
    /// callers simply receive the complete `Vec`.
    pub fn affected_applications(&self) -> Result<Vec<AffectedApplication>> {
        todo!()
    }
}
