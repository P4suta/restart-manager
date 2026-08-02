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

The release workflow requires replacing `Unreleased` with a dated
`## [0.1.0] - YYYY-MM-DD` heading on the commit that receives the annotated
`v0.1.0` tag.
