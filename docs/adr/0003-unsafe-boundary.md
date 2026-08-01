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
module is the single place allowed to opt back in with
`#![allow(unsafe_code)]`; it exposes a minimal, already-safe interface that
the rest of the crate consumes.

## Consequences

- Safety review concentrates on one file.
- Everything above `sys` — including the entire public API — is auditable as
  ordinary safe Rust.
- Raw handles, pointers, callbacks, UTF-16 buffers, and `windows-sys` types do
  not cross the adapter boundary.
