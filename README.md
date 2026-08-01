# restart-manager

Safe blocking Rust bindings for the Windows Restart Manager (`rstrtmgr.dll`).
The crate finds applications using files, processes, or services, can ask them
to shut down, and can restart applications that support restart.

The API is architecture-first: primary and joined installer roles are separate
types, every session operation needs `&mut self`, native handles never escape,
and all `unsafe` code is confined to the private Win32 adapter.

## Quick start

```rust
use restart_manager::{RestartSession, ShutdownOptions};

fn main() -> Result<(), restart_manager::Error> {
    let mut session = RestartSession::new()?;
    session.register_files([r"C:\some\locked\file.dll"])?;

    let report = session.affected_applications()?;
    for application in &report {
        println!(
            "locked by: {} ({:?})",
            application.display_name().to_string_lossy(),
            application.application_type(),
        );
    }

    session.shutdown_with_options(ShutdownOptions::default())?;
    // Replace, move, or update the registered files here.
    session.restart()?;
    Ok(())
} // Drop always ends the session.
```

`shutdown()` is the graceful shorthand. Forced termination is deliberately
opt-in:

```rust
# use restart_manager::ShutdownOptions;
let options = ShutdownOptions::new().with_force_if_hung(true);
```

Forced shutdown can lose unsaved data in target applications. Do not enable it
as a routine fallback without telling the user.

## Installer roles

`RestartSession` is the primary installer. It owns shutdown, restart,
cancellation, and filter capabilities. `JoinedSession` is a secondary installer
and can only register resources and query the shared affected-application
report.

```rust
# use restart_manager::{JoinedSession, RestartSession};
let primary = RestartSession::new()?;
let serialized = primary.session_key().as_str().to_owned();

// Transfer `serialized` to the cooperating process through trusted IPC.
let key = serialized.parse()?;
let mut joined = JoinedSession::join(&key)?;
joined.register_files([r"C:\product\component.dll"])?;
# Ok::<(), restart_manager::Error>(())
```

A `SessionKey` accepts exactly 32 ASCII hexadecimal characters. `Debug` redacts
its value; `as_str()` and `Display` expose it only when requested explicitly.

## Batch registration

`ResourceSet` combines all resource kinds into one `RmRegisterResources` call:

```rust
# use restart_manager::{ResourceSet, RestartSession, UniqueProcess};
let current = UniqueProcess::current()?;
let resources = ResourceSet::new()
    .file(r"C:\product\app.exe")
    .process(current)
    .service("EventLog");

let mut session = RestartSession::new()?;
session.register_resources(&resources)?;
# Ok::<(), restart_manager::Error>(())
```

The convenience methods `register_files`, `register_processes`, and
`register_services` are available on both session roles.

File paths may be relative. They are made absolute at registration time, but
the crate does not resolve symlinks, canonicalize, or check existence.
Restart Manager does not support directory registration; these APIs are for
files and executable paths only. Embedded NULs and native count overflows are
rejected before FFI.

## Reports and forward compatibility

`affected_applications()` returns an iterable `AffectedApplications` snapshot.
It keeps both the entries and `RebootReasons`, so repeated iteration cannot
silently lose the reboot information.

- Display and service names are `OsStr`/`OsString`, preserving all Windows
  UTF-16 data without lossy conversion.
- Invalid process and Terminal Services identifiers become `Option::None`.
- `ApplicationStatus` is a bitmask and retains unknown bits. Windows status
  values are an OR-able history, not a single enum state.
- `ApplicationType::Unknown` is the documented `RmUnknownApp` value;
  `ApplicationType::Unrecognized(raw)` preserves future values.
- `ApplicationType::Critical` means Windows reported `RmCritical`; it should
  not be interpreted as only the process criticality flag.

## Filters

Filters apply only to the primary installer:

```rust
# use restart_manager::{FilterAction, FilterTarget, RestartSession};
# let mut session = RestartSession::new()?;
let target = FilterTarget::executable(r"C:\product\do-not-restart.exe");
session.set_filter(target.clone(), FilterAction::PreventRestart)?;

for filter in session.filters()? {
    println!("{:?}: {:?}", filter.target(), filter.action());
}
session.remove_filter(&target)?;
# Ok::<(), restart_manager::Error>(())
```

The two Windows actions are represented exactly:

- `PreventRestart`: shutdown is allowed, but restart is masked.
- `PreventShutdown`: both shutdown and restart are masked.

Targets are an executable's full path, an exact `UniqueProcess`, or a service
short name.

## Progress and cancellation

The native callback has no context pointer, so callback-bearing operations use
one process-global lease. A second concurrent callback operation fails
immediately with `ErrorKind::CallbackInUse`. Callbacks are `FnMut(Progress) +
Send`; values are bounded to `0..=100` and made non-decreasing. A callback panic
is caught at the FFI boundary and resumed after the Win32 call returns.

Use `CancellationHandle` for the one operation intentionally allowed from a
different thread:

```rust
# use restart_manager::RestartSession;
# let mut session = RestartSession::new()?;
let cancellation = session.cancellation_handle();
std::thread::scope(|scope| {
    scope.spawn(move || {
        // A Ctrl-C handler or supervisor can own this clone.
        let _ = cancellation.cancel();
    });
    let _ = session.shutdown();
});
# Ok::<(), restart_manager::Error>(())
```

The capability uses weak ownership, so retaining it never consumes one of
Windows' 64 session slots. During `cancel()` it temporarily pins the session and
serializes against `RmEndSession`.

## Shutdown and restart limitations

- Graceful shutdown can fail when an application ignores the Windows shutdown
  protocol. Inspect the refreshed application statuses after failure.
- `with_force_if_hung(true)` may terminate unresponsive programs and lose data.
- Restart works only for services and applications that called
  `RegisterApplicationRestart`; Restart Manager cannot recreate arbitrary
  processes.
- `with_only_registered(true)` prevents any shutdown unless every affected
  application can be restarted.
- Windows respects user and Terminal Services session boundaries. Elevated or
  service installers cannot use Restart Manager to control applications in a
  different interactive session.
- `RmCritical` and non-empty `RebootReasons` can mean a system reboot is needed
  before work can safely continue.

## Errors and lifecycle

`Error` has a private representation. `Error::kind()` is the stable semantic
classification and `Error::raw_os_error()` preserves a Win32 code when one
exists.

Dropping either session role calls `RmEndSession` exactly once. Call `end(self)`
only when an explicit end error must be reported. RAII is important because
Windows permits only 64 concurrent Restart Manager sessions per user session.

## Platform and MSRV

The public API exists only on Windows. The crate deliberately compiles with no
public items on non-Windows targets so platform-generic workspaces can still run
`cargo check` and documentation tooling.

- Version: 0.1.0
- Edition: Rust 2024
- MSRV: Rust 1.88
- Runtime model: blocking only
- Normal dependencies: `windows-sys`, `thiserror`, and `bitflags`

## Repository checks

On Windows:

```text
cargo test --workspace --all-targets -- --test-threads=1
cargo xtask e2e
cargo clippy --workspace --all-targets --all-features -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
cargo publish --dry-run
```

`cargo xtask e2e` creates only dedicated fixture processes and temporary files.
It verifies exclusive lock detection, graceful file release, registered
restart, progress, filter suppression, cross-thread cancellation, and cleanup
on all exit paths.

Architecture decisions live in [`docs/adr`](docs/adr). The repository CI also
checks MSRV/stable, Windows 2022/latest, x86_64/i686/aarch64 compile targets,
non-Windows compilation, public API changes, dependency policy, package
consumption, and at least 90% line/region/function coverage.

## License

Licensed under either Apache-2.0 or MIT, at your option.
