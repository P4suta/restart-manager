# Changelog

All notable changes are recorded here.

## [1.0.0] - 2026-08-02

### Added

- Owned shutdown/restart recovery typestates with automatic Drop recovery.
- Portable public API and stable unsupported-platform errors.
- Application-side restart registration with an exclusive process lease.
- Optional Tokio facade with dedicated FIFO workers and coalescing progress.
- Operation-specific error kinds, raw HRESULT access, and post-operation reports.

### Changed

- Renamed `UniqueProcess` to `ProcessIdentity`.
- Renamed `ResourceSet` to `ResourceBatch`.
- Restricted joined sessions to registration, key access, and end.
- Made filter targets opaque, validated, borrowed, and path-stable.
- Replaced `with_only_registered` with
  `with_require_restart_registration`.
- Removed `SessionKey::Display` and introduced `ParseSessionKeyError`.

### Removed

- Reusable shutdown/restart sequencing on `RestartSession`.
- The Windows-only public API cfg surface from 0.1.
