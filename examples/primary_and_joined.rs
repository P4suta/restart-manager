#[cfg(windows)]
fn main() -> Result<(), restart_manager::Error> {
    use restart_manager::{JoinedSession, RestartSession, SessionKey};
    use std::str::FromStr;

    let primary = RestartSession::new()?;
    let transferred = primary.session_key().as_str().to_owned();
    let key = SessionKey::from_str(&transferred)?;
    let mut joined = JoinedSession::join(&key)?;
    joined.register_processes(&[restart_manager::UniqueProcess::current()?])?;

    println!("joined session contains {}", joined.session_key());
    joined.end()?;
    primary.end()
}

#[cfg(not(windows))]
fn main() {
    eprintln!("this example requires Windows");
}
