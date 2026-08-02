# ADR 0005: Owned recovery typestate

Status: accepted for the initial 0.1.0 release.

Restart Manager requires installers to call `RmRestart` even after
`RmShutdown` reports partial failure. A reusable mutable session makes an
early return between those calls easy.

Shutdown therefore consumes `RestartSession` and returns `RestartPending`
for both native success and failure. The pending value retains the shutdown
outcome. Only that type can restart or explicitly leave applications stopped.
Its destructor makes a best-effort restart and then lets the native session
owner end.

`RestartPending` uses ownership of its optional session core as the recovery
arm; no separate boolean can disagree with that ownership. Operation outcomes
are moved into the completion state rather than cloned.

Callback-lease and unsupported-platform failures occur before native work and
return `OperationNotStarted<T>` with the original owned state. Recovery
completion retains shutdown and restart outcomes independently and cannot
return to the registration state.

`OperationNotStarted<T>` implements `Display` and `std::error::Error`. The
Tokio facade uses a separate `AsyncOperationError<T>` because a disconnected
worker cannot safely promise that an owned typestate is reusable: callback
lease conflicts carry `Some(state)`, while worker and internal-state failures
carry `None`.

Automatic rollback, file replacement, process killing, and a generic workflow
DSL remain application concerns.
