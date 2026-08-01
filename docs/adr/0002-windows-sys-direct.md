# ADR 0002: Depend on `windows-sys` directly, not `windows`

- Status: accepted
- Date: 2026-08-01

## Context

The Restart Manager surface is a handful of flat C functions and structs in
`rstrtmgr.dll`. The `windows` crate brings COM/WinRT machinery and a heavier
build for convenience wrappers we would immediately hide behind our own safe
API anyway.

## Decision

Bind through `windows-sys 0.61` with only the `Win32_Foundation` and
`Win32_System_RestartManager` features enabled — the same judgement call
made for conpty-oxide.

## Consequences

- Minimal compile-time cost; raw types never appear in the public API.
- We own the thin safe layer instead of relying on generated wrappers.
