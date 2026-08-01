#![cfg(windows)]

use std::str::FromStr;

use restart_manager::{
    ErrorKind, FilterAction, FilterTarget, JoinedSession, ProcessIdentity, ResourceBatch,
    RestartSession, SessionKey,
};

#[test]
fn start_join_batch_register_query_and_explicit_end() {
    let mut primary = RestartSession::new().unwrap();
    let text = primary.session_key().as_str().to_owned();
    let parsed = SessionKey::from_str(&text).unwrap();
    let mut joined = JoinedSession::join(&parsed).unwrap();

    let current = ProcessIdentity::current().unwrap();
    assert_eq!(current.pid(), std::process::id());
    assert_eq!(ProcessIdentity::from_pid(current.pid()).unwrap(), current);

    let resources = ResourceBatch::new()
        .file(std::env::current_exe().unwrap())
        .process(current);
    joined.register_resources(&resources).unwrap();
    joined
        .register_files([std::env::current_exe().unwrap()])
        .unwrap();
    joined.register_processes([current]).unwrap();
    joined.register_services(["EventLog"]).unwrap();

    primary
        .register_resources(&ResourceBatch::new().process(current))
        .unwrap();
    primary.register_processes([current]).unwrap();
    primary.register_services(["EventLog"]).unwrap();

    let report = primary.affected_applications().unwrap();
    assert!(report.iter().any(|application| {
        application
            .process()
            .is_some_and(|p| p.pid() == current.pid())
    }));

    joined.end().unwrap();
    primary.end().unwrap();
}

#[test]
fn primary_filter_round_trip_and_weak_cancellation() {
    let mut session = RestartSession::new().unwrap();
    let cancellation = session.cancellation_handle();
    let target = FilterTarget::executable(std::env::current_exe().unwrap()).unwrap();

    session
        .set_filter(&target, FilterAction::PreventRestart)
        .unwrap();
    let filters = session.filters().unwrap();
    assert!(filters.iter().any(|filter| {
        filter.target() == &target && filter.action() == FilterAction::PreventRestart
    }));
    session.remove_filter(&target).unwrap();
    assert!(
        !session
            .filters()
            .unwrap()
            .iter()
            .any(|filter| filter.target() == &target)
    );

    session.end().unwrap();
    assert_eq!(
        cancellation.cancel().unwrap_err().kind(),
        ErrorKind::SessionEnded
    );
}

#[test]
fn drop_releases_the_fixed_session_budget() {
    for _ in 0..70 {
        drop(RestartSession::new().unwrap());
    }
}

#[test]
fn recovery_typestate_retains_both_outcomes_and_drop_releases_slot() {
    let session = RestartSession::new().unwrap();
    let pending = session.shutdown();
    assert!(pending.shutdown_outcome().is_success());
    let completion = pending.restart();
    assert!(completion.outcome().shutdown_outcome().is_success());
    assert!(
        completion
            .outcome()
            .restart_outcome()
            .is_some_and(|outcome| outcome.is_success())
    );
    completion.end().unwrap();

    drop(RestartSession::new().unwrap().shutdown());
    RestartSession::new().unwrap().end().unwrap();
}

#[test]
fn public_application_restart_lease_is_exclusive_and_updatable() {
    use restart_manager::{ApplicationRestartOptions, ApplicationRestartRegistration};

    let mut registration =
        ApplicationRestartRegistration::register(ApplicationRestartOptions::new("--first"))
            .unwrap();
    let duplicate =
        ApplicationRestartRegistration::register(ApplicationRestartOptions::new("--duplicate"))
            .unwrap_err();
    assert_eq!(duplicate.kind(), ErrorKind::ApplicationRestartInUse);
    registration
        .update(
            ApplicationRestartOptions::new("--second")
                .with_restart_on_crash(false)
                .with_restart_on_update(false),
        )
        .unwrap();
    registration.unregister().unwrap();

    ApplicationRestartRegistration::register(ApplicationRestartOptions::default())
        .unwrap()
        .unregister()
        .unwrap();
}

#[test]
fn directory_registration_has_a_stable_error_kind() {
    let directory = std::env::temp_dir().join(format!(
        "restart-manager-directory-test-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir(&directory);
    std::fs::create_dir(&directory).unwrap();
    let mut session = RestartSession::new().unwrap();
    let error = session.register_files([&directory]).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::DirectoryNotSupported);
    session.end().unwrap();
    std::fs::remove_dir(directory).unwrap();
}

#[test]
fn cancellation_and_end_race_does_not_leak_or_panic() {
    for _ in 0..25 {
        let session = RestartSession::new().unwrap();
        let cancellation = session.cancellation_handle();
        let cancel = std::thread::spawn(move || cancellation.cancel());
        let _ = session.end();
        let _ = cancel.join().expect("cancellation thread must not panic");
    }
    RestartSession::new().unwrap().end().unwrap();
}

#[test]
fn all_filter_target_kinds_round_trip() {
    let mut session = RestartSession::new().unwrap();
    let process = ProcessIdentity::current().unwrap();
    let process_target = FilterTarget::process(process);
    let service_target = FilterTarget::service("EventLog").unwrap();

    session
        .set_filter(&process_target, FilterAction::PreventRestart)
        .unwrap();
    session
        .set_filter(&service_target, FilterAction::PreventShutdown)
        .unwrap();
    let filters = session.filters().unwrap();
    assert!(
        filters
            .iter()
            .any(|filter| filter.target() == &process_target)
    );
    assert!(
        filters
            .iter()
            .any(|filter| filter.target() == &service_target)
    );
    session.remove_filter(&process_target).unwrap();
    session.remove_filter(&service_target).unwrap();
    session.end().unwrap();
}
