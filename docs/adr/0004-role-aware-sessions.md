# ADR 0004: Role-aware sessions and capability-based cancellation

- Status: accepted
- Date: 2026-08-01

## Context

Restart Manager distinguishes the primary installer that starts a session from
secondary installers that join it. Windows rejects primary-only operations from
a secondary installer. A single public session type would make those invalid
calls representable and defer the role error until runtime.

Long-running shutdown and restart calls also need cancellation from another
thread. Giving every operation shared access would make concurrent register,
query, shutdown, and end calls possible on the same native handle.

## Decision

Expose `RestartSession` for the primary installer and `JoinedSession` for a
secondary installer. Both own a private `SessionCore`; only the primary type
publishes reporting, shutdown, filters, and cancellation. The joined role can
only register resources, inspect its key, and end.

Registration, reporting, and filter operations take `&mut self`. Shutdown
consumes the primary state and enters the recovery typestate from ADR 0005.
Cancellation is a separate
`CancellationHandle` capability containing weak ownership of the private native
handle. Upgrading that weak reference pins the native session only for the
duration of `cancel()`. Native cancellation and explicit end serialize inside
the private handle owner.

## Consequences

- A secondary installer cannot compile code that performs a primary-only call.
- Rust borrowing excludes ordinary same-handle concurrency without a public
  busy state or general-purpose operation mutex.
- Retaining a cancellation capability does not retain one of the 64 native
  session slots.
- Explicit end marks the handle ended only after native success; destruction
  makes one best-effort retry after an explicit failure.
