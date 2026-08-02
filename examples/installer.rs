//! Complete blocking installer/update recipe.

#[cfg(windows)]
use restart_manager::{RestartSession, ShutdownOptions};

#[cfg(windows)]
#[derive(Debug, thiserror::Error)]
enum InstallerError {
    #[error("Restart Manager operation failed: {0}")]
    RestartManager(#[from] restart_manager::Error),
    #[error("file update failed: {0}")]
    Update(#[from] std::io::Error),
}

#[cfg(windows)]
type InstallerResult<T = ()> = Result<T, InstallerError>;

#[cfg(windows)]
fn main() -> InstallerResult {
    let Some(file) = std::env::args_os().nth(1) else {
        eprintln!("usage: installer <file-to-update>");
        return Ok(());
    };

    let mut session = RestartSession::new()?;
    session.register_files([&file])?;

    let report = session.affected_applications()?;
    eprintln!("reboot reasons: {:?}", report.reboot_reasons());
    for application in &report {
        eprintln!(
            "{} (restartable: {})",
            application.display_name().to_string_lossy(),
            application.is_restartable(),
        );
    }

    let pending = session
        .shutdown_with_options(ShutdownOptions::new().with_require_restart_registration(true));

    // Never use `?` between shutdown and restart. Keep the update result so
    // restart is attempted even when replacement fails.
    let update_result = if pending.shutdown_outcome().is_success() {
        replace_registered_file(&file)
    } else {
        Ok(())
    };

    // Microsoft requires an RmRestart attempt even after RmShutdown fails.
    let completion = pending.restart();
    let outcome = completion.outcome().clone();
    let end_result = completion.end();

    // Every operation has now been attempted; report each retained result.
    outcome.shutdown_outcome().clone().into_result()?;
    update_result?;
    outcome
        .restart_outcome()
        .expect("this recipe always attempts restart")
        .clone()
        .into_result()?;
    end_result?;
    Ok(())
}

#[cfg(windows)]
fn replace_registered_file(_file: &std::ffi::OsStr) -> std::io::Result<()> {
    // Perform the installer's atomic file replacement or rollback here.
    Ok(())
}

#[cfg(not(windows))]
fn main() {
    eprintln!("this example requires Windows");
}
