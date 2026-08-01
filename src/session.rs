//! Session lifetime: start, join, register resources, end (RAII).

use std::fmt;
use std::path::Path;
use std::str::FromStr;

use crate::application::UniqueProcess;
use crate::error::{Error, Result};

/// The textual key that identifies a Restart Manager session across process
/// boundaries.
///
/// `RmStartSession` hands out two identifiers: a numeric handle (valid only
/// inside the starting process) and this key. Pass the key to a cooperating
/// process — over a command line, an environment variable, IPC — so it can
/// attach to the same session with [`RestartSession::join`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SessionKey(String);

impl SessionKey {
    /// Returns the key as a string slice, ready to hand to another process.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SessionKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for SessionKey {
    type Err = Error;

    /// Parses a key previously obtained from
    /// [`RestartSession::session_key`] in another process.
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        let _ = s;
        todo!()
    }
}

/// An open Restart Manager session, released automatically on `Drop`.
///
/// # Why RAII
///
/// Windows allows at most **64 concurrent Restart Manager sessions per user
/// session**. A leaked session is not a private leak: it eats into a
/// machine-wide budget shared with every installer and updater running for
/// that user. Tying `RmEndSession` to `Drop` makes the release path
/// unconditional — early returns, `?`, and panics included.
///
/// # Typical flow
///
/// 1. [`RestartSession::new`] (or [`RestartSession::join`]),
/// 2. `register_*` the resources you care about,
/// 3. [`affected_applications`](Self::affected_applications) to see who holds them,
/// 4. [`shutdown`](Self::shutdown), do your file work, [`restart`](Self::restart).
pub struct RestartSession {
    /// Numeric session handle returned by `RmStartSession`; every other
    /// `Rm*` call takes it.
    #[allow(dead_code)] // read once the FFI layer (`sys` module) lands
    handle: u32,
    /// Key for cross-process [`join`](Self::join).
    key: SessionKey,
}

impl RestartSession {
    /// Starts a new Restart Manager session (`RmStartSession`).
    ///
    /// # Errors
    ///
    /// Returns [`Error::SessionLimit`] when all 64 per-user-session slots
    /// are taken, or another [`Error`] variant for other Win32 failures.
    pub fn new() -> Result<Self> {
        todo!()
    }

    /// Attaches to a session started by another process (`RmJoinSession`).
    ///
    /// The joining process gains the same view of registered resources and
    /// may register more of them.
    pub fn join(key: &SessionKey) -> Result<Self> {
        let _ = key;
        todo!()
    }

    /// Returns the key that other processes can use to [`join`](Self::join)
    /// this session.
    pub fn session_key(&self) -> &SessionKey {
        &self.key
    }

    /// Registers files or directories whose lockers you want to find
    /// (`RmRegisterResources` with file paths).
    ///
    /// May be called repeatedly; registrations accumulate for the lifetime
    /// of the session.
    pub fn register_paths(
        &mut self,
        paths: impl IntoIterator<Item = impl AsRef<Path>>,
    ) -> Result<()> {
        let _ = paths;
        todo!()
    }

    /// Registers concrete processes (`RmRegisterResources` with
    /// `RM_UNIQUE_PROCESS` entries).
    pub fn register_processes(&mut self, processes: &[UniqueProcess]) -> Result<()> {
        let _ = processes;
        todo!()
    }

    /// Registers Windows services by their short (key) names
    /// (`RmRegisterResources` with service names).
    pub fn register_services(&mut self, services: &[impl AsRef<str>]) -> Result<()> {
        let _ = services;
        todo!()
    }
}

impl Drop for RestartSession {
    /// Ends the session (`RmEndSession`).
    ///
    /// Errors from `RmEndSession` cannot be surfaced from `drop` and are
    /// intentionally discarded; nothing actionable can be done with them at
    /// this point.
    fn drop(&mut self) {
        todo!()
    }
}
