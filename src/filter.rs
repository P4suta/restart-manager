//! Typed shutdown and restart filters.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use crate::UniqueProcess;

/// The executable, process, or service selected by a filter.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FilterTarget {
    /// An executable's full path. Directories are not supported.
    Executable(PathBuf),
    /// One exact process identity.
    Process(UniqueProcess),
    /// A Windows service short name.
    Service(OsString),
}

impl FilterTarget {
    /// Creates an executable-path target.
    #[must_use]
    pub fn executable(path: impl Into<PathBuf>) -> Self {
        Self::Executable(path.into())
    }

    /// Creates a process target.
    #[must_use]
    pub const fn process(process: UniqueProcess) -> Self {
        Self::Process(process)
    }

    /// Creates a service target.
    #[must_use]
    pub fn service(name: impl Into<OsString>) -> Self {
        Self::Service(name.into())
    }

    /// Returns the executable path when this is an executable filter.
    #[must_use]
    pub fn as_executable(&self) -> Option<&Path> {
        match self {
            Self::Executable(path) => Some(path),
            Self::Process(_) | Self::Service(_) => None,
        }
    }

    /// Returns the process when this is a process filter.
    #[must_use]
    pub const fn as_process(&self) -> Option<UniqueProcess> {
        match self {
            Self::Process(process) => Some(*process),
            Self::Executable(_) | Self::Service(_) => None,
        }
    }

    /// Returns the service short name when this is a service filter.
    #[must_use]
    pub fn as_service(&self) -> Option<&OsStr> {
        match self {
            Self::Service(name) => Some(name),
            Self::Executable(_) | Self::Process(_) => None,
        }
    }
}

/// The official `RM_FILTER_ACTION` behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FilterAction {
    /// Permit shutdown but prevent a later restart (`RmNoRestart`).
    PreventRestart,
    /// Prevent both shutdown and restart (`RmNoShutdown`).
    PreventShutdown,
}

/// One filter returned by [`crate::RestartSession::filters`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Filter {
    pub(crate) target: FilterTarget,
    pub(crate) action: FilterAction,
}

impl Filter {
    /// Returns the selected resource.
    #[must_use]
    pub const fn target(&self) -> &FilterTarget {
        &self.target
    }

    /// Returns the modification applied to that resource.
    #[must_use]
    pub const fn action(&self) -> FilterAction {
        self.action
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_constructors_and_accessors_are_disjoint() {
        let executable = FilterTarget::executable("demo.exe");
        assert_eq!(executable.as_executable(), Some(Path::new("demo.exe")));
        assert_eq!(executable.as_process(), None);
        assert_eq!(executable.as_service(), None);

        let process = UniqueProcess::from_parts(1, 2);
        let process_target = FilterTarget::process(process);
        assert_eq!(process_target.as_process(), Some(process));
        assert_eq!(process_target.as_executable(), None);
        assert_eq!(process_target.as_service(), None);

        let service = FilterTarget::service("EventLog");
        assert_eq!(service.as_service(), Some(OsStr::new("EventLog")));
        assert_eq!(service.as_executable(), None);
        assert_eq!(service.as_process(), None);

        let filter = Filter {
            target: service,
            action: FilterAction::PreventShutdown,
        };
        assert_eq!(filter.action(), FilterAction::PreventShutdown);
        assert!(matches!(filter.target(), FilterTarget::Service(_)));
    }
}
