//! Shared validation for operating-system strings and paths.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use crate::{Error, ErrorKind, Result};

pub(crate) fn contains_nul(value: &OsStr) -> bool {
    value.as_encoded_bytes().contains(&0)
}

pub(crate) fn validate_user_os_value(value: &OsStr, description: &'static str) -> Result<()> {
    validate_os_value(value, description, ErrorKind::InvalidInput)
}

pub(crate) fn validate_os_output(value: &OsStr, description: &'static str) -> Result<()> {
    validate_os_value(value, description, ErrorKind::MalformedOsData)
}

fn validate_os_value(value: &OsStr, description: &'static str, kind: ErrorKind) -> Result<()> {
    if value.is_empty() {
        return Err(Error::new(
            kind,
            None,
            format!("{description} may not be empty"),
        ));
    }
    if contains_nul(value) {
        return Err(Error::new(
            kind,
            None,
            format!("{description} may not contain an embedded NUL"),
        ));
    }
    Ok(())
}

pub(crate) fn absolute_user_path(
    path: impl AsRef<Path>,
    description: &'static str,
) -> Result<PathBuf> {
    let path = path.as_ref();
    validate_user_os_value(path.as_os_str(), description)?;
    std::path::absolute(path).map_err(|error| {
        Error::new(
            ErrorKind::InvalidInput,
            error.raw_os_error().map(|code| code as u32),
            format!("{description} could not be made absolute"),
        )
    })
}

pub(crate) fn validate_absolute_os_path(path: &Path, description: &'static str) -> Result<()> {
    validate_os_output(path.as_os_str(), description)?;
    if !path.is_absolute() {
        return Err(Error::new(
            ErrorKind::MalformedOsData,
            None,
            format!("Windows returned a relative {description}"),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nul_detection_and_error_origin_are_preserved() {
        assert!(!contains_nul(OsStr::new("valid")));
        assert!(contains_nul(OsStr::new("bad\0value")));
        assert_eq!(
            validate_user_os_value(OsStr::new(""), "a value")
                .unwrap_err()
                .kind(),
            ErrorKind::InvalidInput
        );
        assert_eq!(
            validate_os_output(OsStr::new("bad\0value"), "a value")
                .unwrap_err()
                .kind(),
            ErrorKind::MalformedOsData
        );
    }

    #[test]
    fn user_paths_are_absolutized_but_os_paths_must_already_be_absolute() {
        let absolute = absolute_user_path("relative.exe", "an executable path").unwrap();
        assert!(absolute.is_absolute());
        assert_eq!(
            validate_absolute_os_path(Path::new("relative.exe"), "filter path")
                .unwrap_err()
                .kind(),
            ErrorKind::MalformedOsData
        );
        validate_absolute_os_path(&absolute, "filter path").unwrap();
    }
}
