# ADR 0001: Blocking-first API; async arrives later as a `tokio` feature

- Status: accepted
- Date: 2026-08-01

## Context

`RmShutdown` and `RmRestart` are long-running, blocking OS calls that report
progress through a callback. The primary consumers — installers, updaters,
CLI tools such as `deno clean` — are synchronous at the point where they
need the Restart Manager.

## Decision

The default API is blocking and depends on no async runtime. The initial 0.1.0
API also offers an optional Tokio facade whose dedicated worker design is
recorded in ADR 0006. Other runtimes can move the Send blocking typestate into
their own blocking facility.

## Consequences

- Zero runtime dependencies for the common installer/CLI case.
- The blocking API is canonical; the optional Tokio layer preserves its
  typestate and recovery semantics.
