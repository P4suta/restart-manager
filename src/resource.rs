//! Resource collections registered with Restart Manager in one batch.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use crate::ProcessIdentity;

/// Files, processes, and services to register in one native call.
#[derive(Debug, Clone, Default)]
pub struct ResourceBatch {
    files: Vec<PathBuf>,
    processes: Vec<ProcessIdentity>,
    services: Vec<OsString>,
}

impl ResourceBatch {
    /// Creates an empty resource collection.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            files: Vec::new(),
            processes: Vec::new(),
            services: Vec::new(),
        }
    }

    /// Adds a file path and returns the collection for chaining.
    ///
    /// Directories are not supported. Relative paths are made absolute at
    /// registration time; other missing paths are retained without
    /// canonicalizing or resolving symlinks.
    #[must_use]
    pub fn file(mut self, file: impl Into<PathBuf>) -> Self {
        self.files.push(file.into());
        self
    }

    /// Adds a process and returns the collection for chaining.
    #[must_use]
    pub fn process(mut self, process: ProcessIdentity) -> Self {
        self.processes.push(process);
        self
    }

    /// Adds a service short name and returns the collection for chaining.
    #[must_use]
    pub fn service(mut self, service: impl Into<OsString>) -> Self {
        self.services.push(service.into());
        self
    }

    /// Adds a file path in place.
    pub fn add_file(&mut self, file: impl Into<PathBuf>) -> &mut Self {
        self.files.push(file.into());
        self
    }

    /// Adds a process in place.
    pub fn add_process(&mut self, process: ProcessIdentity) -> &mut Self {
        self.processes.push(process);
        self
    }

    /// Adds a service short name in place.
    pub fn add_service(&mut self, service: impl Into<OsString>) -> &mut Self {
        self.services.push(service.into());
        self
    }

    /// Returns whether no resources have been added.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.files.is_empty() && self.processes.is_empty() && self.services.is_empty()
    }

    /// Returns the number of resources across all three categories.
    #[must_use]
    pub fn len(&self) -> usize {
        self.files.len() + self.processes.len() + self.services.len()
    }

    pub(crate) fn files(&self) -> impl Iterator<Item = &Path> {
        self.files.iter().map(PathBuf::as_path)
    }

    pub(crate) fn processes(&self) -> &[ProcessIdentity] {
        &self.processes
    }

    pub(crate) fn services(&self) -> impl Iterator<Item = &OsStr> {
        self.services.iter().map(OsString::as_os_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_and_mutating_forms_cover_every_resource_kind() {
        let process = ProcessIdentity::from_raw_parts(1, 2).unwrap();
        let resources = ResourceBatch::new()
            .file("one.txt")
            .process(process)
            .service("EventLog");
        assert_eq!(resources.len(), 3);
        assert!(!resources.is_empty());
        assert_eq!(
            resources.files().collect::<Vec<_>>(),
            [Path::new("one.txt")]
        );
        assert_eq!(resources.processes(), &[process]);
        assert_eq!(
            resources.services().collect::<Vec<_>>(),
            [OsStr::new("EventLog")]
        );

        let mut resources = ResourceBatch::default();
        assert!(resources.is_empty());
        resources
            .add_file("two.txt")
            .add_process(process)
            .add_service("Schedule");
        assert_eq!(resources.len(), 3);
    }
}
