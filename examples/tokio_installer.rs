//! Complete Tokio installer/update recipe.

#[cfg(windows)]
use std::error::Error;

#[cfg(windows)]
use restart_manager::ShutdownOptions;
#[cfg(windows)]
use restart_manager::tokio::{ProgressReceiver, RestartSession};

#[cfg(windows)]
type DynResult<T = ()> = Result<T, Box<dyn Error>>;

#[cfg(windows)]
#[tokio::main(flavor = "current_thread")]
async fn main() -> DynResult {
    let Some(file) = std::env::args_os().nth(1) else {
        eprintln!("usage: tokio_installer <file-to-update>");
        return Ok(());
    };

    let mut session = RestartSession::new().await?;
    session.register_files([&file]).await?;

    let report = session.affected_applications().await?;
    eprintln!("reboot reasons: {:?}", report.reboot_reasons());
    for application in &report {
        eprintln!(
            "{} (restartable: {})",
            application.display_name().to_string_lossy(),
            application.is_restartable(),
        );
    }

    let (shutdown, shutdown_progress) = session
        .shutdown_with_progress(ShutdownOptions::new().with_require_restart_registration(true));
    let shutdown_reporter = tokio::spawn(report_progress("shutdown", shutdown_progress));
    let pending = match shutdown.await {
        Ok(pending) => pending,
        Err(not_started) => return Err(not_started.into_parts().1.into()),
    };
    shutdown_reporter.await?;

    // Retain this result: returning here would skip the explicit restart call.
    let update_result = if pending.shutdown_outcome().is_success() {
        replace_registered_file(&file).await
    } else {
        Ok(())
    };

    let (restart, restart_progress) = pending.restart_with_progress();
    let restart_reporter = tokio::spawn(report_progress("restart", restart_progress));
    let completion = match restart.await {
        Ok(completion) => completion,
        Err(not_started) => return Err(not_started.into_parts().1.into()),
    };
    restart_reporter.await?;

    let outcome = completion.outcome().clone();
    let end_result = completion.end().await;

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
async fn report_progress(operation: &'static str, mut progress: ProgressReceiver) {
    while let Some(progress) = progress.recv().await {
        eprintln!("{operation}: {}%", progress.percent_complete());
    }
}

#[cfg(windows)]
async fn replace_registered_file(_file: &std::ffi::OsStr) -> DynResult {
    // Perform the installer's async file replacement or rollback here.
    Ok(())
}

#[cfg(not(windows))]
fn main() {
    eprintln!("this example requires Windows");
}
