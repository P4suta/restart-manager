# Changelog

All notable changes are recorded here.

## [Unreleased]

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

The release workflow requires a dated `## [0.1.0] - YYYY-MM-DD` heading on the
commit that receives the annotated `v0.1.0` tag. The `Unreleased` heading is
kept for future changes.
