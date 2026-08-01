# ADR 0005: Owned recovery typestate

Status: accepted for 1.0.

Restart Manager requires installers to call `RmRestart` even after
`RmShutdown` reports partial failure. A reusable mutable session makes an
early return between those calls easy.

Shutdown therefore consumes `RestartSession` and returns `RestartPending`
for both native success and failure. The pending value retains the shutdown
outcome. Only that type can restart or explicitly leave applications stopped.
Its destructor makes a best-effort restart and then lets the native session
owner end.

Callback-lease and unsupported-platform failures occur before native work and
return `OperationNotStarted<T>` with the original owned state. Recovery
completion retains shutdown and restart outcomes independently and cannot
return to the registration state.

Automatic rollback, file replacement, process killing, and a generic workflow
DSL remain application concerns.
