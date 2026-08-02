# restart-manager

[![CI](https://github.com/P4suta/restart-manager/actions/workflows/ci.yml/badge.svg)](https://github.com/P4suta/restart-manager/actions/workflows/ci.yml)
[![CodeQL](https://github.com/P4suta/restart-manager/actions/workflows/codeql.yml/badge.svg)](https://github.com/P4suta/restart-manager/actions/workflows/codeql.yml)

Safe, human-friendly Restart Manager workflows for Rust applications that need
to find, stop, and restart the Windows processes locking their files.

## Installation

```console
cargo add restart-manager
```

The default API is blocking. Enable the optional Tokio API when needed:

```console
cargo add restart-manager --features tokio
```

## Example

Inspect the applications affected by replacing a file:

```rust,no_run
use restart_manager::RestartSession;

fn main() -> Result<(), restart_manager::Error> {
    let mut session = RestartSession::new()?;
    session.register_files([r"C:\product\component.dll"])?;

    let applications = session.affected_applications()?;
    eprintln!("reboot reasons: {:?}", applications.reboot_reasons());
    for application in &applications {
        eprintln!(
            "{} (restartable: {})",
            application.display_name().to_string_lossy(),
            application.is_restartable(),
        );
    }

    session.end()
}
```

## Important behavior

- Native operations are Windows-only. Public types remain available on other
  targets, where OS operations return `ErrorKind::UnsupportedPlatform`.
- Shutdown produces an owned `RestartPending` recovery value. It must be
  restarted or explicitly completed with `leave_stopped`; dropping it makes
  one best-effort restart attempt before ending the session.
- Forced shutdown is opt-in and can cause applications to lose data.
- Restart Manager can relaunch only services and applications that registered
  themselves for restart.

## More information

- [API documentation](https://docs.rs/restart-manager)
- [Blocking examples](examples/basic.rs) and [installer workflow](examples/installer.rs)
- [Tokio installer example](examples/tokio_installer.rs)
- [Changelog](CHANGELOG.md)
- [Security policy](SECURITY.md)
- [Contributing](CONTRIBUTING.md)

## License

Licensed under either [MIT](LICENSE-MIT) or
[Apache-2.0](LICENSE-APACHE), at your option.
