use std::str::FromStr;

use restart_manager::{
    ApplicationRestartOptions, FilterTarget, ProcessIdentity, ResourceBatch, SessionKey,
};

#[test]
fn pure_domain_validation_is_portable() {
    assert!(ProcessIdentity::from_raw_parts(0, 1).is_err());
    assert!(ProcessIdentity::from_raw_parts(u32::MAX, 1).is_err());
    assert!(ProcessIdentity::from_raw_parts(7, 1).is_ok());

    assert!(FilterTarget::executable("").is_err());
    assert!(FilterTarget::service("").is_err());
    assert!(FilterTarget::service("bad\0service").is_err());
    assert!(
        FilterTarget::executable("relative.exe")
            .unwrap()
            .as_executable()
            .unwrap()
            .is_absolute()
    );

    let key = SessionKey::from_str("0123456789abcdef0123456789ABCDEF").unwrap();
    assert_eq!(key.as_str().len(), 32);
    assert!(!format!("{key:?}").contains(key.as_str()));

    let batch = ResourceBatch::new();
    assert!(batch.is_empty());
    assert_eq!(batch.len(), 0);

    let options = ApplicationRestartOptions::new("sensitive");
    assert!(!format!("{options:?}").contains("sensitive"));
}

#[cfg(not(windows))]
#[test]
fn os_operations_are_classified_as_unsupported() {
    use restart_manager::{ErrorKind, RestartSession};

    match RestartSession::new() {
        Err(error) => assert_eq!(error.kind(), ErrorKind::UnsupportedPlatform),
        Ok(_) => panic!("a non-Windows session unexpectedly started"),
    }
    assert_eq!(
        ProcessIdentity::from_pid(1).unwrap_err().kind(),
        ErrorKind::UnsupportedPlatform
    );
}
