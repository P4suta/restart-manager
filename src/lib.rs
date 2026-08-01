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
//! session.shutdown_with_options(ShutdownOptions::default())?;
//! // Replace or update the registered files here.
//! session.restart()?;
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
//! On non-Windows targets the crate compiles but exposes no public items.
#![deny(unsafe_code)]

#[cfg(windows)]
mod application;
#[cfg(windows)]
mod error;
#[cfg(windows)]
mod filter;
#[cfg(windows)]
mod resource;
#[cfg(windows)]
mod session;
#[cfg(windows)]
mod shutdown;
#[cfg(windows)]
mod sys;

#[cfg(windows)]
pub use crate::{
    application::{
        AffectedApplication, AffectedApplications, ApplicationStatus, ApplicationType,
        RebootReasons, UniqueProcess,
    },
    error::{Error, ErrorKind, Result},
    filter::{Filter, FilterAction, FilterTarget},
    resource::ResourceSet,
    session::{CancellationHandle, JoinedSession, RestartSession, SessionKey},
    shutdown::{Progress, ShutdownOptions},
};
