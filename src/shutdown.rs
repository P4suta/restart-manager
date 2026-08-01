//! Stopping and restarting the applications that hold your resources.

use crate::error::Result;
use crate::session::RestartSession;

/// How hard [`RestartSession::shutdown`] may push, mirroring the
/// `RmShutdown` action flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ShutdownPolicy {
    /// Graceful only: `WM_CLOSE` / console control events / service stop
    /// requests. Applications that hang are left running and surface as
    /// [`ApplicationStatus::ErrorOnStop`](crate::ApplicationStatus::ErrorOnStop).
    #[default]
    Graceful,
    /// Try graceful first, then force-terminate whatever does not respond
    /// (`RmForceShutdown`). Data loss in the target is possible; this is a
    /// deliberate, opt-in escalation.
    ForceIfHung,
}

/// A progress report delivered to the `*_with_progress` callbacks,
/// corresponding to the Restart Manager's `RM_WRITE_STATUS_CALLBACK`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    /// Percent of the current operation completed, `0..=100`.
    pub percent_complete: u32,
}

impl RestartSession {
    /// Shuts down every affected application, honouring `policy`
    /// (`RmShutdown`).
    ///
    /// Blocks until the Restart Manager finishes or fails. See
    /// [`shutdown_with_progress`](Self::shutdown_with_progress) for progress
    /// reporting, and [`cancel`](Self::cancel) for interruption.
    pub fn shutdown(&mut self, policy: ShutdownPolicy) -> Result<()> {
        let _ = policy;
        todo!()
    }

    /// Like [`shutdown`](Self::shutdown), invoking `progress` as the
    /// Restart Manager reports advancement.
    pub fn shutdown_with_progress(
        &mut self,
        policy: ShutdownPolicy,
        progress: impl FnMut(Progress),
    ) -> Result<()> {
        let _ = (policy, progress);
        todo!()
    }

    /// Restarts the applications that were shut down by this session and
    /// support being restarted (`RmRestart`).
    ///
    /// Call it after your file operations succeed, so users get their
    /// editors, services, and Explorer windows back.
    pub fn restart(&mut self) -> Result<()> {
        todo!()
    }

    /// Like [`restart`](Self::restart), invoking `progress` as the Restart
    /// Manager reports advancement.
    pub fn restart_with_progress(&mut self, progress: impl FnMut(Progress)) -> Result<()> {
        let _ = progress;
        todo!()
    }

    /// Cancels the shutdown or restart currently in progress in this session
    /// (`RmCancelCurrentTask`).
    ///
    /// Takes `&self` deliberately: the point is to call it from somewhere
    /// else (another thread, a Ctrl-C handler) while a blocking
    /// [`shutdown`](Self::shutdown) or [`restart`](Self::restart) is running.
    pub fn cancel(&self) -> Result<()> {
        todo!()
    }
}
