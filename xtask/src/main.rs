//! Repository automation for native Restart Manager scenarios.

type XtaskResult<T = ()> = Result<T, XtaskError>;

#[derive(Debug, thiserror::Error)]
enum XtaskError {
    #[error("{0}")]
    Message(String),
    #[cfg(not(windows))]
    #[error("{0} requires Windows")]
    Unsupported(&'static str),
    #[error("unknown xtask command: {0}")]
    UnknownCommand(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Time(#[from] std::time::SystemTimeError),
    #[cfg(windows)]
    #[error(transparent)]
    RestartManager(#[from] restart_manager::Error),
}

impl From<String> for XtaskError {
    fn from(message: String) -> Self {
        Self::Message(message)
    }
}

impl From<&str> for XtaskError {
    fn from(message: &str) -> Self {
        Self::Message(message.to_owned())
    }
}

fn main() -> XtaskResult {
    let command = std::env::args().nth(1).unwrap_or_else(|| "help".to_owned());
    match command.as_str() {
        "e2e" => e2e(),
        "fixture" => fixture(),
        "fixture-restarted" => fixture_restarted(),
        "help" | "--help" | "-h" => {
            println!("cargo xtask e2e    run the isolated Windows end-to-end scenario");
            Ok(())
        }
        other => Err(XtaskError::UnknownCommand(other.to_owned())),
    }
}

#[cfg(not(windows))]
fn e2e() -> XtaskResult {
    Err(XtaskError::Unsupported("the Restart Manager E2E scenario"))
}

#[cfg(not(windows))]
fn fixture() -> XtaskResult {
    Err(XtaskError::Unsupported("the Restart Manager fixture"))
}

#[cfg(not(windows))]
fn fixture_restarted() -> XtaskResult {
    Err(XtaskError::Unsupported("the Restart Manager fixture"))
}

#[cfg(windows)]
fn e2e() -> XtaskResult {
    windows::e2e()
}

#[cfg(windows)]
fn fixture() -> XtaskResult {
    windows::fixture()
}

#[cfg(windows)]
fn fixture_restarted() -> XtaskResult {
    windows::fixture_restarted()
}

#[cfg(windows)]
mod windows {
    use std::fs::{self, OpenOptions};
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::process::CommandExt;
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, Stdio};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    use restart_manager::{
        ApplicationRestartOptions, ApplicationRestartRegistration, ApplicationStatus, ErrorKind,
        FilterAction, FilterTarget, ProcessIdentity, RestartSession, ShutdownOptions,
    };
    use windows_sys::Win32::System::Console::{
        CTRL_BREAK_EVENT, CTRL_CLOSE_EVENT, CTRL_LOGOFF_EVENT, CTRL_SHUTDOWN_EVENT,
        SetConsoleCtrlHandler,
    };
    use windows_sys::Win32::System::Threading::{
        CREATE_NEW_CONSOLE, CREATE_NEW_PROCESS_GROUP, ExitProcess,
    };

    use super::XtaskResult;

    static IGNORE_SHUTDOWN: AtomicBool = AtomicBool::new(false);

    pub(super) fn e2e() -> XtaskResult {
        let unique = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "restart-manager-e2e-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&directory)?;
        let lock_path = directory.join("locked.file");
        let ready_path = directory.join("ready");
        let restart_path = directory.join("restarted");
        fs::write(&lock_path, b"restart-manager isolated fixture")?;

        let executable = std::env::current_exe()?;
        let child = Command::new(&executable)
            .arg("fixture")
            .arg(&lock_path)
            .arg(&ready_path)
            .arg(&restart_path)
            // A separate console is essential: Restart Manager's console
            // shutdown event must never reach cargo, the shell, or the agent
            // hosting this E2E run.
            .creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NEW_CONSOLE)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        let child_pid = child.id();
        let mut cleanup = Cleanup::new(child, directory.clone());
        wait_for_path(&ready_path, Duration::from_secs(10))?;

        let process = ProcessIdentity::from_pid(child_pid)?;
        let mut session = RestartSession::new()?;
        session.register_files([&lock_path])?;
        let report = session.affected_applications()?;
        if !report
            .iter()
            .any(|application| application.process().is_some_and(|p| p == process))
        {
            return Err("Restart Manager did not report the isolated file locker".into());
        }

        let target = FilterTarget::process(process);
        session.set_filter(&target, FilterAction::PreventShutdown)?;
        let pending = session.shutdown();
        if cleanup.child_mut().try_wait()?.is_some() {
            return Err("PreventShutdown filter did not keep the fixture alive".into());
        }
        let mut completion = pending.restart();
        let masked = completion
            .affected_applications()?
            .iter()
            .any(|application| {
                application.process().is_some_and(|p| p == process)
                    && application
                        .status()
                        .contains(ApplicationStatus::SHUTDOWN_MASKED)
            });
        if !masked {
            return Err("affected report did not retain the shutdown-masked status bit".into());
        }
        completion.end()?;

        let mut session = RestartSession::new()?;
        session.register_files([&lock_path])?;
        let mut shutdown_progress = Vec::new();
        let pending = match session.shutdown_with_progress(ShutdownOptions::default(), |progress| {
            shutdown_progress.push(progress.percent_complete());
        }) {
            Ok(pending) => pending,
            Err(not_started) => return Err(not_started.error().to_string().into()),
        };
        pending.shutdown_outcome().clone().into_result()?;
        wait_for_exit(cleanup.child_mut(), Duration::from_secs(10))?;
        assert_progress("shutdown", &shutdown_progress)?;

        let mut restart_progress = Vec::new();
        let completion = match pending.restart_with_progress(|progress| {
            restart_progress.push(progress.percent_complete());
        }) {
            Ok(completion) => completion,
            Err(not_started) => return Err(not_started.error().to_string().into()),
        };
        if let Some(restart) = completion.outcome().restart_outcome() {
            restart.clone().into_result()?;
        }
        wait_for_path(&restart_path, Duration::from_secs(10))?;
        assert_progress("restart", &restart_progress)?;
        completion.end()?;

        cleanup.finish();
        cancellation_e2e(&executable)?;
        println!(
            "E2E passed: exclusive lock detection, graceful release, restart, progress, filter suppression, and cross-thread cancellation"
        );
        Ok(())
    }

    pub(super) fn fixture() -> XtaskResult {
        let mut arguments = std::env::args_os().skip(2);
        let lock_path = required_path(arguments.next(), "lock path")?;
        let ready_path = required_path(arguments.next(), "ready path")?;
        let restart_path = required_path(arguments.next(), "restart marker path")?;
        let ignore_shutdown = arguments
            .next()
            .is_some_and(|value| value == "ignore-shutdown");
        IGNORE_SHUTDOWN.store(ignore_shutdown, Ordering::Release);

        let _locked_file = OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(0)
            .open(&lock_path)?;
        // SAFETY: the handler has the required system ABI and remains present
        // until this dedicated process exits.
        if unsafe { SetConsoleCtrlHandler(Some(console_handler), 1) } == 0 {
            return Err(std::io::Error::last_os_error().into());
        }

        let restart_command = format!("fixture-restarted \"{}\"", restart_path.display());
        let _registration = ApplicationRestartRegistration::register(
            ApplicationRestartOptions::new(restart_command),
        )?;
        fs::write(ready_path, b"ready")?;

        loop {
            thread::park_timeout(Duration::from_secs(60));
        }
    }

    pub(super) fn fixture_restarted() -> XtaskResult {
        let marker = required_path(std::env::args_os().nth(2), "restart marker path")?;
        fs::write(marker, b"restarted")?;
        Ok(())
    }

    unsafe extern "system" fn console_handler(event: u32) -> i32 {
        if matches!(
            event,
            CTRL_BREAK_EVENT | CTRL_CLOSE_EVENT | CTRL_LOGOFF_EVENT | CTRL_SHUTDOWN_EVENT
        ) {
            if IGNORE_SHUTDOWN.load(Ordering::Acquire) {
                return 1;
            }
            // SAFETY: terminating only this isolated fixture process is the
            // intended response to Restart Manager's console shutdown event.
            unsafe { ExitProcess(0) }
        }
        0
    }

    fn cancellation_e2e(executable: &Path) -> XtaskResult {
        let unique = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "restart-manager-cancel-e2e-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&directory)?;
        let lock_path = directory.join("locked.file");
        let ready_path = directory.join("ready");
        let restart_path = directory.join("unused-restart-marker");
        fs::write(&lock_path, b"restart-manager cancellation fixture")?;
        let child = Command::new(executable)
            .arg("fixture")
            .arg(&lock_path)
            .arg(&ready_path)
            .arg(&restart_path)
            .arg("ignore-shutdown")
            .creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NEW_CONSOLE)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        let child_pid = child.id();
        let mut cleanup = Cleanup::new(child, directory);
        wait_for_path(&ready_path, Duration::from_secs(10))?;

        let mut session = RestartSession::new()?;
        session.register_files([&lock_path])?;
        if !session.affected_applications()?.iter().any(|application| {
            application
                .process()
                .is_some_and(|process| process.pid() == child_pid)
        }) {
            return Err("cancellation fixture was not detected as the file locker".into());
        }
        let cancellation = session.cancellation_handle();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let operation = thread::spawn(move || {
            let mut notified = false;
            session.shutdown_with_progress(
                ShutdownOptions::new().with_force_if_hung(true),
                move |_| {
                    if !notified {
                        notified = true;
                        let _ = started_tx.send(());
                    }
                },
            )
        });
        started_rx
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| "shutdown returned before reporting cancellable progress")?;
        cancellation.cancel()?;
        let result = operation
            .join()
            .map_err(|_| "cancellation shutdown thread panicked")?;
        let pending = match result {
            Ok(pending) => pending,
            Err(error) => {
                return Err(format!("shutdown did not start: {}", error.error()).into());
            }
        };
        match pending.shutdown_outcome().error() {
            Some(error) if error.kind() == ErrorKind::Cancelled => {}
            Some(error) => {
                return Err(
                    format!("shutdown returned the wrong cancellation error: {error}").into(),
                );
            }
            None => return Err("shutdown completed instead of observing cancellation".into()),
        }
        if cleanup.child_mut().try_wait()?.is_some() {
            return Err("cancelled shutdown unexpectedly terminated its fixture".into());
        }
        cleanup.finish();
        Ok(())
    }

    fn required_path(value: Option<std::ffi::OsString>, name: &str) -> XtaskResult<PathBuf> {
        value
            .map(PathBuf::from)
            .ok_or_else(|| format!("fixture is missing {name}").into())
    }

    fn wait_for_path(path: &Path, timeout: Duration) -> XtaskResult {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if path.exists() {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(50));
        }
        Err(format!("timed out waiting for {}", path.display()).into())
    }

    fn wait_for_exit(child: &mut Child, timeout: Duration) -> XtaskResult {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if let Some(status) = child.try_wait()? {
                // Console applications stopped by Restart Manager commonly
                // report STATUS_CONTROL_C_EXIT (0xC000013A), not exit code 0.
                // RmShutdown already supplied the authoritative operation
                // result; this check only waits for handle/file release.
                let _ = status;
                return Ok(());
            }
            thread::sleep(Duration::from_millis(50));
        }
        Err("timed out waiting for the fixture to release its file".into())
    }

    fn assert_progress(operation: &str, values: &[u8]) -> XtaskResult {
        if values.is_empty() {
            return Err(format!("{operation} did not report progress").into());
        }
        if values.iter().any(|value| *value > 100)
            || values.windows(2).any(|pair| pair[0] >= pair[1])
        {
            return Err(format!("{operation} progress was out of range or decreased").into());
        }
        Ok(())
    }

    struct Cleanup {
        child: Option<Child>,
        directory: PathBuf,
    }

    impl Cleanup {
        fn new(child: Child, directory: PathBuf) -> Self {
            Self {
                child: Some(child),
                directory,
            }
        }

        fn child_mut(&mut self) -> &mut Child {
            self.child.as_mut().expect("fixture child is present")
        }

        fn finish(&mut self) {
            self.cleanup();
        }

        fn cleanup(&mut self) {
            if let Some(mut child) = self.child.take() {
                match child.try_wait() {
                    Ok(Some(_)) => {}
                    Ok(None) | Err(_) => {
                        let _ = child.kill();
                        let _ = child.wait();
                    }
                }
            }
            let _ = fs::remove_dir_all(&self.directory);
        }
    }

    impl Drop for Cleanup {
        fn drop(&mut self) {
            self.cleanup();
        }
    }
}
