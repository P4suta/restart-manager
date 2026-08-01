#![cfg(windows)]

use std::str::FromStr;

use restart_manager::{
    ErrorKind, FilterAction, FilterTarget, JoinedSession, ResourceSet, RestartSession, SessionKey,
    UniqueProcess,
};

#[test]
fn start_join_batch_register_query_and_explicit_end() {
    let mut primary = RestartSession::new().unwrap();
    let text = primary.session_key().as_str().to_owned();
    let parsed = SessionKey::from_str(&text).unwrap();
    let mut joined = JoinedSession::join(&parsed).unwrap();

    let current = UniqueProcess::current().unwrap();
    assert_eq!(current.pid(), std::process::id());
    assert_eq!(UniqueProcess::from_pid(current.pid()).unwrap(), current);

    let resources = ResourceSet::new()
        .file(std::env::current_exe().unwrap())
        .process(current);
    joined.register_resources(&resources).unwrap();
    joined
        .register_files([std::env::current_exe().unwrap()])
        .unwrap();
    joined.register_processes(&[current]).unwrap();
    joined.register_services(["EventLog"]).unwrap();

    primary
        .register_resources(&ResourceSet::new().process(current))
        .unwrap();
    primary.register_processes(&[current]).unwrap();
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
    let target = FilterTarget::executable(std::env::current_exe().unwrap());

    session
        .set_filter(target.clone(), FilterAction::PreventRestart)
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
