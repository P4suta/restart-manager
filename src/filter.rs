//! Per-resource overrides of the default shutdown/restart behaviour.

use std::path::PathBuf;

use crate::application::UniqueProcess;
use crate::error::Result;
use crate::session::RestartSession;

/// A resource that a filter applies to.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FilterResource {
    /// A file or directory path.
    Path(PathBuf),
    /// A specific process (PID + start time).
    Process(UniqueProcess),
    /// A Windows service, by short (key) name.
    Service(String),
}

/// The override applied to a [`FilterResource`], mirroring
/// `RM_FILTER_ACTION`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FilterAction {
    /// Leave the resource completely alone: neither shut it down nor
    /// restart it.
    Noop,
    /// Shut the resource down, but do not restart it afterwards.
    Shutdown,
    /// Shut the resource down and restart it afterwards.
    Restart,
}

impl RestartSession {
    /// Overrides what [`shutdown`](Self::shutdown) and
    /// [`restart`](Self::restart) will do to one resource (`RmAddFilter`).
    ///
    /// Typical use: keep a database service running while everything else is
    /// cycled, or prevent the restart of a tool you are about to replace.
    pub fn add_filter(&mut self, resource: FilterResource, action: FilterAction) -> Result<()> {
        let _ = (resource, action);
        todo!()
    }

    /// Removes a previously added override, restoring the default behaviour
    /// for that resource (`RmRemoveFilter`).
    pub fn remove_filter(&mut self, resource: &FilterResource) -> Result<()> {
        let _ = resource;
        todo!()
    }
}
