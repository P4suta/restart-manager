//! Safe blocking bindings to the Windows Restart Manager (`rstrtmgr.dll`).
//!
//! A primary [`RestartSession`] owns shutdown, restart, filter, and
//! cancellation capabilities. A secondary [`JoinedSession`] can only register
//! and inspect resources, making a role violation impossible to express.
//!
//! # Example
//!
//! ```rust,no_run
//! # #[cfg(windows)]
//! # fn main() -> Result<(), restart_manager::Error> {
//! use restart_manager::{RestartSession, ShutdownOptions};
//!
//! let mut session = RestartSession::new()?;
//! session.register_files([r"C:\some\locked\file.dll"])?;
//! let report = session.affected_applications()?;
//! for application in &report {
//!     println!("locked by: {:?}", application.display_name());
//! }
//! let pending = session.shutdown_with_options(ShutdownOptions::default());
//! // Replace or update the registered files here.
//! let completion = pending.restart();
//! completion.end()?;
//! # Ok(())
//! # }
//! # #[cfg(not(windows))]
//! # fn main() {}
//! ```
//!
//! Relative file and executable paths are made absolute, but this crate never
//! checks existence or canonicalizes them. Restart Manager does not support
//! registering directories. Forced shutdown is opt-in and can lose target
//! application data. Restart is only possible for services and applications
//! that registered for restart.
//!
//! Progress callbacks use a process-global native callback slot because the
//! Windows API supplies no context pointer. Concurrent callback-bearing calls
//! fail immediately with [`ErrorKind::CallbackInUse`]. A callback panic is
//! contained at the FFI boundary and resumed after Windows returns.
//!
//! # Platform support
//!
//! All domain and session types are available on every target. Pure input
//! validation behaves identically everywhere; operations that require Windows
//! return [`ErrorKind::UnsupportedPlatform`].
//!
//! # Typestate guarantees
//!
//! A joined installer cannot query or control the primary workflow:
//!
//! ```compile_fail
//! fn invalid(joined: &mut restart_manager::JoinedSession) {
//!     let _ = joined.affected_applications();
//! }
//! ```
//!
//! Restart is not available before a shutdown attempt:
//!
//! ```compile_fail
//! fn invalid(session: restart_manager::RestartSession) {
//!     let _ = session.restart();
//! }
//! ```
//!
//! A pending recovery state cannot register resources, manipulate filters, or
//! end the native session:
//!
//! ```compile_fail
//! fn invalid(mut pending: restart_manager::RestartPending) {
//!     let batch = restart_manager::ResourceBatch::new();
//!     let _ = pending.register_resources(&batch);
//!     let _ = pending.end();
//! }
//! ```
//!
//! Consuming a state prevents a second operation on the same value:
//!
//! ```compile_fail
//! fn invalid(session: restart_manager::RestartSession) {
//!     let _pending = session.shutdown();
//!     let _ = session.end();
//! }
//! ```
#![deny(unsafe_code)]

mod application;
mod application_restart;
mod error;
mod filter;
mod resource;
mod session;
mod shutdown;
mod sys;
#[cfg(feature = "tokio")]
pub mod tokio;

pub use crate::{
    application::{
        AffectedApplication, AffectedApplications, ApplicationStatus, ApplicationType,
        ProcessIdentity, RebootReasons,
    },
    application_restart::{ApplicationRestartOptions, ApplicationRestartRegistration},
    error::{Error, ErrorKind, ParseSessionKeyError, Result},
    filter::{Filter, FilterAction, FilterTarget},
    resource::ResourceBatch,
    session::{
        CancellationHandle, JoinedSession, OperationNotStarted, RecoveryCompletion, RestartPending,
        RestartSession, SessionKey,
    },
    shutdown::{OperationOutcome, Progress, RecoveryOutcome, ShutdownOptions},
};
