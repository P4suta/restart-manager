//! Safe, human-friendly bindings to the **Windows Restart Manager**
//! (`rstrtmgr.dll`).
//!
//! # Mission
//!
//! Windows refuses to touch files that other processes hold open — the
//! infamous *"The action can't be completed because the file is open in
//! another program"*. Since Windows Vista the operating system has shipped a
//! first-party answer, the Restart Manager, yet using it from Rust today
//! means hand-writing `unsafe` calls against raw bindings. This crate wraps
//! the whole session lifecycle in one safe RAII type so you can:
//!
//! * find out **which processes lock** a set of files, services, or processes,
//! * **shut them down** gracefully (force only by explicit choice),
//! * **restart** the ones that support being restarted, once you are done.
//!
//! # Quick start
//!
//! ```rust,no_run
//! use restart_manager::{RestartSession, ShutdownPolicy};
//!
//! fn main() -> Result<(), restart_manager::Error> {
//!     let mut session = RestartSession::new()?;
//!     session.register_paths([r"C:\some\locked\file.dll"])?;
//!
//!     for app in session.affected_applications()? {
//!         println!("locked by: {} ({:?})", app.display_name, app.app_type);
//!     }
//!
//!     session.shutdown(ShutdownPolicy::Graceful)?;
//!     // ... replace / move / delete the files here ...
//!     session.restart()?;
//!     Ok(())
//! } // `Drop` ends the session (`RmEndSession`) even on early return.
//! ```
//!
//! # Platform support
//!
//! This crate is **Windows-only**. On other targets it still compiles but
//! exposes no items (the entire API sits behind `#[cfg(windows)]`), so
//! platform-generic tooling — a Linux CI running `cargo check`, workspace-wide
//! doc builds, and the like — keeps working without `compile_error!` tripwires.
#![deny(unsafe_code)]
// Unsafe policy (see docs/adr/0003-unsafe-boundary.md): the FFI layer will
// live in a dedicated `src/sys.rs` module, which alone will opt back in via
// `#![allow(unsafe_code)]`. Everything else — including the whole public
// API — stays 100% safe Rust.

#[cfg(windows)]
mod application;
#[cfg(windows)]
mod error;
#[cfg(windows)]
mod filter;
#[cfg(windows)]
mod session;
#[cfg(windows)]
mod shutdown;

#[cfg(windows)]
pub use crate::{
    application::{AffectedApplication, ApplicationStatus, ApplicationType, UniqueProcess},
    error::{Error, Result},
    filter::{FilterAction, FilterResource},
    session::{RestartSession, SessionKey},
    shutdown::{Progress, ShutdownPolicy},
};
