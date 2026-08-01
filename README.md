# restart-manager

Safe, human-friendly Rust bindings for the **Windows Restart Manager**
(`rstrtmgr.dll`): find out which processes lock your files, shut them down
gracefully, and restart them afterwards — all through a single RAII session
type.

> **Status:** API scaffold. Types, signatures, and documentation are in
> place; implementations land next.

## Why this crate should exist

*"The action can't be completed because the file is open in another
program."* Windows has shipped the first-party cure for this since Vista —
the Restart Manager — but the Rust ecosystem never wrapped it:

- **Nobody on crates.io wraps the Restart Manager.** As of 2026-08-01,
  exact-name lookups (`restart-manager` and variations) all return 404,
  keyword searches return zero crates, and `winsafe` does not cover the API.
  What exists are the raw `unsafe` bindings in `windows` / `windows-sys` —
  nothing more.
- **Well-known projects keep re-inventing the same dangerous code
  independently:**
  - **Deno** hand-writes `unsafe` calls to `RmStartSession` / `RmGetList` in
    [`cli/util/windows.rs`](https://github.com/denoland/deno/blob/main/cli/util/windows.rs)
    so that `deno clean` can tell users *which* process is locking the cache.
  - **Mullvad VPN** hand-rolled its own RAII guard, `RMSession`, in
    [`mullvad-nsis/src/handle.rs`](https://github.com/mullvad/mullvadvpn-app/blob/main/mullvad-nsis/src/handle.rs)
    for its installer — complete with lingering `NOTE: Ignoring error here`
    and `TODO: consider letting RestartManager stop the service` comments.
- **The raw API has footguns that belong behind a library boundary:**
  - Windows allows at most **64 Restart Manager sessions per user session**.
    Sessions are a machine-wide (per user session) budget shared by every
    installer and updater; leak them and the feature stops working. Hence
    the RAII design: `Drop` always calls `RmEndSession`.
  - `RmGetList` requires the classic `ERROR_MORE_DATA` two-phase buffer
    dance. This crate performs it internally and hands you a `Vec`.

## Quick start

```rust
use restart_manager::{RestartSession, ShutdownPolicy};

fn main() -> Result<(), restart_manager::Error> {
    let mut session = RestartSession::new()?;
    session.register_paths([r"C:\some\locked\file.dll"])?;

    for app in session.affected_applications()? {
        println!("locked by: {} ({:?})", app.display_name, app.app_type);
    }

    session.shutdown(ShutdownPolicy::Graceful)?;
    // ... replace / move / delete the files here ...
    session.restart()?;
    Ok(())
} // `Drop` ends the session (`RmEndSession`) even on early return.
```

## Non-goals

- **NT handle enumeration.** Finding lockers *without* the Restart Manager —
  walking open handles NT-style — is the territory of the
  [`filelocksmith`](https://crates.io/crates/filelocksmith) crate (velopack).
  Different mechanism, different trade-offs, deliberately out of scope here.
- **GUI or end-user tooling.** This is a library.
- **A generic force-kill utility.** Processes are only ever terminated
  through the Restart Manager's own, explicit policy
  (`ShutdownPolicy::ForceIfHung`).

## Roadmap

- **v0.1** — blocking core: RAII session (`RmStartSession` /
  `RmJoinSession` / `RmEndSession`), resource registration (paths,
  processes, services), affected-application listing, shutdown/restart with
  progress callbacks, per-resource filters.
- **Later** — an opt-in `tokio` feature layering async wrappers over the
  blocking core (see `docs/adr/0001-blocking-first.md`).

## Platform support and MSRV

Windows-only by nature. On other targets the crate still compiles but
exposes no items, so cross-platform workspaces and CI keep working.

Minimum supported Rust version: **1.88**.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in the work by you, as defined in the Apache-2.0
license, shall be dual licensed as above, without any additional terms or
conditions.
