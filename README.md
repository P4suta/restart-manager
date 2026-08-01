# restart-manager

Safe Restart Manager workflows for Rust.

Version 1.0 turns the Windows Restart Manager API into an owned recovery
protocol: once shutdown is attempted, the only normal choices are restart or
an explicit decision to leave applications stopped.

## Core workflow

```rust,no_run
use restart_manager::{OperationOutcome, RestartSession, ShutdownOptions};

fn update_file() -> Result<(), restart_manager::Error> {
    let mut session = RestartSession::new()?;
    session.register_files([r"C:\product\component.dll"])?;

    for application in &session.affected_applications()? {
        eprintln!(
            "{} (restartable: {})",
            application.display_name().to_string_lossy(),
            application.is_restartable()
        );
    }

    let pending = session.shutdown_with_options(
        ShutdownOptions::new().with_require_restart_registration(true),
    );

    if let OperationOutcome::Failed(error) = pending.shutdown_outcome() {
        let error = error.clone();
        pending.restart().end()?;
        return Err(error);
    }

    // Replace the registered files here.

    let completion = pending.restart();
    if let Some(restart) = completion.outcome().restart_outcome() {
        restart.clone().into_result()?;
    }
    completion.end()?;
    Ok(())
}
```

`RestartSession` has no `restart` method. Consuming shutdown produces
`RestartPending`, which alone has `restart`, `restart_with_progress`, and
`leave_stopped`. Shutdown and restart outcomes are retained independently, so
a restart failure never overwrites a preceding shutdown failure.

Dropping an armed `RestartPending` makes one best-effort restart attempt and
then ends the session. This protects early `?`, panics, and partially
successful shutdown. `leave_stopped` is the only explicit opt-out.
`mem::forget`, process abort, and `process::exit` do not run destructors and
are outside this guarantee.

## Resources and reports

`ResourceBatch` groups files, exact process identities, and service short
names into one native registration call. Duplicates are retained. Registering
an empty batch is a successful no-op.

```rust,no_run
use restart_manager::{ProcessIdentity, ResourceBatch, RestartSession};

# fn run() -> Result<(), restart_manager::Error> {
let process = ProcessIdentity::current()?;
let batch = ResourceBatch::new()
    .file(r"C:\product\component.dll")
    .process(process)
    .service("EventLog");
let mut session = RestartSession::new()?;
session.register_resources(&batch)?;
# session.end()
# }
```

`ProcessIdentity::from_raw_parts` rejects PID 0 and the native invalid
sentinel. Its creation-time value and getter are explicitly measured in
100-nanosecond units since 1601-01-01 UTC.

`AffectedApplications` and `AffectedApplication` are reusable,
`PartialEq + Eq` reports. Names remain `OsString`, unknown status bits are
retained, future application kinds remain lossless, and invalid native process
or terminal-session IDs become `None`.

## Primary and joined roles

A primary `RestartSession` owns reporting, filters, shutdown, cancellation,
and recovery. A secondary `JoinedSession` can only register resources,
inspect the session key, and end its joined handle. This follows the secondary
installer protocol and prevents a joined installer from driving the primary
workflow.

`SessionKey` uses a dedicated `ParseSessionKeyError`, redacts `Debug`, and
does not implement `Display`. Transfer it only through explicit `as_str` or
`into_string`.

## Filters

`FilterTarget` is opaque and validated. Executable paths are made absolute
when the target is created, so changing the working directory between
`set_filter` and `remove_filter` does not change filter identity. Empty
values and embedded NULs are rejected.

```rust,no_run
# use restart_manager::{FilterAction, FilterTarget, RestartSession};
# fn run() -> Result<(), restart_manager::Error> {
let mut session = RestartSession::new()?;
let target = FilterTarget::executable(r"C:\product\app.exe")?;
session.set_filter(&target, FilterAction::PreventRestart)?;
session.remove_filter(&target)?;
# session.end()
# }
```

## Application-side restart registration

`RmRestart` can recreate an application only if that application registered
itself. The process-side capability is public:

```rust,no_run
use restart_manager::{
    ApplicationRestartOptions, ApplicationRestartRegistration,
};

# fn run() -> Result<(), restart_manager::Error> {
let options = ApplicationRestartOptions::new("--restore-session")
    .with_restart_on_crash(false);
let mut registration = ApplicationRestartRegistration::register(options)?;
registration.update(ApplicationRestartOptions::new("--restore-session=2"))?;
registration.unregister()?;
# Ok(())
# }
```

Arguments do not include the executable name. They are lossless `OsString`
values, redacted from `Debug`, and validated for embedded NULs and the
1024 UTF-16 code-unit limit before FFI. A process-global lease rejects duplicate
crate-managed ownership. Explicit unregister failure stays armed so destruction
makes one best-effort retry.

## Progress and cancellation

Blocking progress callbacks receive validated `Progress` values in strictly
increasing order within `0..=100`. Invalid native samples are suppressed and
reported as `MalformedOsData`. Only one callback-bearing operation can own the
process-global native callback slot; another returns
`OperationNotStarted<T>` with both the original typestate and
`ErrorKind::CallbackInUse`.

Callback panics are contained at the ABI boundary. Priority is deterministic:
callback panic first, then native error, then progress-protocol error.

`CancellationHandle` is weak and thread-safe, so keeping it does not retain
one of the 64 native session slots.

## Tokio feature

The default API has no async runtime dependency. Enable `tokio` for
`restart_manager::tokio::{RestartSession, JoinedSession, RestartPending,
RecoveryCompletion}`.

Each async session owns one dedicated standard worker thread. That worker alone
owns the blocking typestate and processes commands in FIFO order. Tokio
`oneshot` channels carry results and `watch` channels carry cloneable,
coalescing progress through `ProgressReceiver`; user callbacks never run on
the worker.

Dropping a shutdown future requests best-effort cancellation, after which the
worker recovers partially stopped applications and ends. Dropping a restart
future detaches it so restart finishes before cleanup. Other runtimes can move
the `Send` blocking typestate into their own blocking facility.

## Platform behavior

All public domain and session types exist on every target. Pure validation is
portable. Calls that require Windows return
`ErrorKind::UnsupportedPlatform`; they are not hidden with target cfg.

The stable `ErrorKind` classification includes platform, registry, reboot,
partial shutdown/restart, sequencing, filter, allocation, application lease,
and async-worker failures. `raw_os_error` preserves Win32 codes and
`raw_hresult` preserves application-restart HRESULTs.

## Limits

- Restart Manager does not accept directories as registered file resources.
- Graceful shutdown can fail when an application ignores its shutdown protocol.
- Forced shutdown is opt-in and can lose application data.
- Restart applies only to services and applications registered for restart.
- Windows user, privilege, and Terminal Services boundaries still apply.
- Forced fallback, process killing, file replacement, rollback, SCM fallback,
  WER recovery callbacks, serde, tracing, and a generic workflow DSL are not
  part of 1.0.

## Compatibility and checks

- Version: 1.0.0
- Edition: Rust 2024
- MSRV: Rust 1.88
- Default runtime model: blocking
- Normal dependencies: `windows-sys`, `thiserror`, `bitflags`, and optional
  `tokio`

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-targets --all-features --locked -- --test-threads=1
cargo test --doc --all-features --locked
cargo xtask e2e
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --locked --no-deps
cargo package --locked
```

See [MIGRATION.md](MIGRATION.md), [CHANGELOG.md](CHANGELOG.md), and
[docs/adr](docs/adr) for the compatibility and design record.

## License

Licensed under either Apache-2.0 or MIT, at your option.
