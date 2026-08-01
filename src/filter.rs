//! Typed shutdown and restart filters.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use crate::{Error, ErrorKind, ProcessIdentity, Result};

/// The executable, process, or service selected by a filter.
///
/// The representation is intentionally opaque. Executable paths are made
/// absolute exactly once during construction, so a later working-directory
/// change cannot alter the identity used to remove the filter.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FilterTarget {
    kind: TargetKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum TargetKind {
    Executable(PathBuf),
    Process(ProcessIdentity),
    Service(OsString),
}

impl FilterTarget {
    /// Creates a validated executable-path target.
    pub fn executable(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        validate_os_value(path.as_os_str(), "an executable path")?;
        let path = std::path::absolute(path).map_err(|error| {
            Error::new(
                ErrorKind::InvalidInput,
                error.raw_os_error().map(|code| code as u32),
                "the executable path could not be made absolute",
            )
        })?;
        Ok(Self {
            kind: TargetKind::Executable(path),
        })
    }

    /// Creates a process target.
    #[must_use]
    pub const fn process(process: ProcessIdentity) -> Self {
        Self {
            kind: TargetKind::Process(process),
        }
    }

    /// Creates a validated service-short-name target.
    pub fn service(name: impl Into<OsString>) -> Result<Self> {
        let name = name.into();
        validate_os_value(&name, "a service short name")?;
        Ok(Self {
            kind: TargetKind::Service(name),
        })
    }

    /// Returns the absolute executable path for an executable filter.
    #[must_use]
    pub fn as_executable(&self) -> Option<&Path> {
        match &self.kind {
            TargetKind::Executable(path) => Some(path),
            TargetKind::Process(_) | TargetKind::Service(_) => None,
        }
    }

    /// Returns the process identity for a process filter.
    #[must_use]
    pub const fn as_process(&self) -> Option<ProcessIdentity> {
        match self.kind {
            TargetKind::Process(process) => Some(process),
            TargetKind::Executable(_) | TargetKind::Service(_) => None,
        }
    }

    /// Returns the service short name for a service filter.
    #[must_use]
    pub fn as_service(&self) -> Option<&OsStr> {
        match &self.kind {
            TargetKind::Service(name) => Some(name),
            TargetKind::Executable(_) | TargetKind::Process(_) => None,
        }
    }

    pub(crate) fn from_raw_executable(path: PathBuf) -> Result<Self> {
        validate_os_value(path.as_os_str(), "an executable path")?;
        Ok(Self {
            kind: TargetKind::Executable(path),
        })
    }

    pub(crate) const fn from_raw_process(process: ProcessIdentity) -> Self {
        Self::process(process)
    }

    pub(crate) fn from_raw_service(name: OsString) -> Result<Self> {
        validate_os_value(&name, "a service short name")?;
        Ok(Self {
            kind: TargetKind::Service(name),
        })
    }
}

fn validate_os_value(value: &OsStr, description: &'static str) -> Result<()> {
    if value.is_empty() {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            None,
            format!("{description} may not be empty"),
        ));
    }
    if value.to_string_lossy().contains('\0') {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            None,
            format!("{description} may not contain an embedded NUL"),
        ));
    }
    Ok(())
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
    fn targets_are_validated_and_disjoint() {
        let executable = FilterTarget::executable("demo.exe").unwrap();
        assert!(executable.as_executable().unwrap().is_absolute());
        assert_eq!(executable.as_process(), None);
        assert_eq!(executable.as_service(), None);

        let process = ProcessIdentity::from_raw_parts(1, 2).unwrap();
        let process_target = FilterTarget::process(process);
        assert_eq!(process_target.as_process(), Some(process));

        let service = FilterTarget::service("EventLog").unwrap();
        assert_eq!(service.as_service(), Some(OsStr::new("EventLog")));
        assert!(FilterTarget::service("").is_err());
        assert!(FilterTarget::executable("").is_err());
        assert!(FilterTarget::service("bad\0name").is_err());
    }
}
