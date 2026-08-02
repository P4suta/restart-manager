# ADR 0003: All `unsafe` lives in the private `sys` module

- Status: accepted
- Date: 2026-08-01

## Context

FFI requires `unsafe`. A scattered unsafe surface is exactly how the
hand-rolled copies of this code in Deno and Mullvad accumulated comments
like "NOTE: Ignoring error here" — every call site re-derives the safety
argument on its own.

## Decision

The crate root carries `#![deny(unsafe_code)]`. The private `src/sys.rs`
module is the single logical boundary allowed to opt back in with
`#![allow(unsafe_code)]`; it exposes a minimal, already-safe interface that
the rest of the crate consumes.

## Consequences

- Safety review concentrates on one file.
- Everything above `sys` — including the entire public API — is auditable as
  ordinary safe Rust.
- Raw handles, pointers, callbacks, UTF-16 buffers, and `windows-sys` types do
  not cross the adapter boundary.
- OS-string validation and path normalization happen once in the safe layer;
  the adapter only performs the mechanical UTF-16 conversion.
- The boundary is organized by lifecycle, input/UTF-16, application lists,
  filter buffers, callbacks, and application restart responsibilities. Its
  unsupported backend contains no unsafe code.
- The borrowed `dyn FnMut` callback and boxed panic payload are the only stored
  type-erased values. Restart Manager's
  [progress callback](https://learn.microsoft.com/en-us/windows/win32/api/restartmanager/nc-restartmanager-rm_write_status_callback)
  has no caller context pointer, so replacing that boundary with a manual
  vtable would increase the unsafe surface.
