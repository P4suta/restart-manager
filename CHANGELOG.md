# Changelog

All notable changes are recorded here.

## [Unreleased]

## [0.1.0] - 2026-08-03

Initial release.

### Added

- Owned shutdown/restart recovery typestates with automatic Drop recovery.
- Portable public API and stable unsupported-platform errors.
- Application-side restart registration with an exclusive process lease.
- Optional Tokio facade with dedicated FIFO workers and coalescing progress.
- Operation-specific error kinds, raw HRESULT access, and post-operation reports.
- Compile-checked blocking and Tokio installer/update recipes.
- Release metadata, package-consumer, SBOM, provenance, and publication checks.
- Guarded Dependabot auto-merge for verified compatible dependency updates.
- Human-reviewed release-plz automation with environment-scoped App credentials.
- Concrete Tokio worker commands without boxed jobs, plus
  `tokio::AsyncOperationError<T>` for explicit optional-state recovery.
- Allocation-free OS-string validation, single-pass safe-layer path
  normalization, and malformed classification for invalid OS filter data.
- Concrete example and xtask error enums without boxed error values.
- REUSE 3.3 licensing metadata (`REUSE.toml` and `LICENSES/`), a `_typos.toml`,
  and a CI job that enforces both. Licensing and spelling were the two baseline
  gates the sibling Windows crates had and this one did not.
- `clippy::undocumented_unsafe_blocks` is denied workspace-wide, so
  CONTRIBUTING's "every `unsafe` block carries a specific safety justification"
  rule is machine-checked. The blocks it flagged were the hardening tests that
  fabricate malformed `RM_FILTER_INFO` records, which is exactly where the
  invariants most needed writing down.

### Known limitations

These are properties of the Windows Restart Manager itself, not of this crate.
They are the reason a caller may find the API less useful than its name
suggests, so they are stated up front rather than discovered in production.

- Restart only reaches services and applications that registered for restart
  through `RegisterApplicationRestart`. Everything else can be shut down but
  will not be brought back; a caller that needs guaranteed recovery must
  restart those processes itself.
- Windows allows at most 64 concurrent Restart Manager sessions machine-wide.
  Exhausting them yields `ErrorKind::SessionLimit`.
- Directories cannot be registered as resources.
- Forced shutdown is opt-in and can lose unsaved data in the target
  applications.
