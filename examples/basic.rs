#[cfg(windows)]
fn main() -> Result<(), restart_manager::Error> {
    use restart_manager::RestartSession;

    let Some(file) = std::env::args_os().nth(1) else {
        eprintln!("usage: basic <file>");
        return Ok(());
    };
    let mut session = RestartSession::new()?;
    session.register_files([file])?;
    let report = session.affected_applications()?;
    println!("reboot reasons: {:?}", report.reboot_reasons());
    for application in &report {
        println!(
            "{} pid={:?} status={:?} restartable={}",
            application.display_name().to_string_lossy(),
            application.process().map(|process| process.pid()),
            application.status(),
            application.is_restartable(),
        );
    }
    Ok(())
}

#[cfg(not(windows))]
fn main() {
    eprintln!("this example requires Windows");
}
