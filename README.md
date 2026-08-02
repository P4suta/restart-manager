# restart-manager

[![CI](https://github.com/P4suta/restart-manager/actions/workflows/ci.yml/badge.svg)](https://github.com/P4suta/restart-manager/actions/workflows/ci.yml)
[![CodeQL](https://github.com/P4suta/restart-manager/actions/workflows/codeql.yml/badge.svg)](https://github.com/P4suta/restart-manager/actions/workflows/codeql.yml)

Safe Restart Manager workflows for Rust.

The crate is designed for Rust developers building Windows installers and
updaters. Its initial, not-yet-published 0.1.0 API turns the Windows Restart
Manager API into an owned recovery protocol: once shutdown is attempted, the
only normal choices are restart or an explicit decision to leave applications
stopped.

## Core workflow

```rust,no_run
use restart_manager::{RestartSession, ShutdownOptions};

fn update_file() -> Result<(), Box<dyn std::error::Error>> {
    let mut session = RestartSession::new()?;
    session.register_files([r"C:\product\component.dll"])?;

    let report = session.affected_applications()?;
    eprintln!("reboot reasons: {:?}", report.reboot_reasons());
    for application in &report {
        eprintln!(
            "{} (restartable: {})",
            application.display_name().to_string_lossy(),
            application.is_restartable()
        );
    }

    let pending = session.shutdown_with_options(
        ShutdownOptions::new().with_require_restart_registration(true),
    );

    // Retain update errors so restart is still attempted.
    let update_result: Result<(), Box<dyn std::error::Error>> =
        if pending.shutdown_outcome().is_success() {
            // Replace the registered files here.
            Ok(())
        } else {
            Ok(())
        };

    let completion = pending.restart();
    let outcome = completion.outcome().clone();
    let end_result = completion.end();

    // All work has been attempted before any retained error is returned.
    outcome.shutdown_outcome().clone().into_result()?;
    update_result?;
    outcome.restart_outcome().unwrap().clone().into_result()?;
    end_result?;
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

The compile-checked [blocking installer example](examples/installer.rs) shows
affected applications and reboot reasons, update gating, restart after every
shutdown attempt and update failure, both retained outcomes, and explicit
session end. This ordering follows Microsoft's
[primary-installer workflow](https://learn.microsoft.com/en-us/windows/win32/rstmgr/using-restart-manager-with-a-primary-installer)
and [RmRestart requirement](https://learn.microsoft.com/en-us/windows/win32/api/restartmanager/nf-restartmanager-rmrestart).

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

`ProgressReceiver::peek` observes without consuming. `recv` waits for a new
coalesced sample and returns `None` when the operation ends normally. Worker
failure is reported by the paired shutdown or restart future as
`ErrorKind::AsyncWorkerUnavailable`, not by the progress stream. See the
compile-checked [Tokio installer example](examples/tokio_installer.rs).

Dropping a shutdown future requests best-effort cancellation, after which the
worker recovers partially stopped applications and ends. Dropping a restart
future detaches it so restart finishes before cleanup. Blocking session
typestates are `Send`, `CancellationHandle` is `Send + Sync`, and every
public Tokio future is `Send`. Other runtimes can move the blocking typestate
into their own blocking facility.

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
  WER recovery callbacks, serde, tracing, a generic workflow DSL, and an
  official CLI are not part of 0.1.0.

## Compatibility and checks

- Version: 0.1.0 (not yet published)
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

See [CHANGELOG.md](CHANGELOG.md) and [docs/adr](docs/adr) for the release and
design record.

## License

Licensed under either Apache-2.0 or MIT, at your option.
