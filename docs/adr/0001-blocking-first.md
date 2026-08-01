# ADR 0001: Blocking-first API; async arrives later as a `tokio` feature

- Status: accepted
- Date: 2026-08-01

## Context

`RmShutdown` and `RmRestart` are long-running, blocking OS calls that report
progress through a callback. The primary consumers — installers, updaters,
CLI tools such as `deno clean` — are synchronous at the point where they
need the Restart Manager.

## Decision

v0.1 exposes a purely blocking API and depends on no async runtime. Async
support is planned as an opt-in `tokio` cargo feature that wraps the
blocking calls (`spawn_blocking`) and forwards progress through a channel.

## Consequences

- Zero runtime dependencies for the common installer/CLI case.
- The blocking API is canonical; the future async layer must add no new
  semantics, only scheduling.
