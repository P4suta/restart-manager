# Migrating from 0.1 to 1.0

Version 1.0 intentionally breaks the provisional 0.1 API.

| 0.1 | 1.0 |
| --- | --- |
| `UniqueProcess` | `ProcessIdentity` |
| `UniqueProcess::from_parts` | fallible `ProcessIdentity::from_raw_parts` |
| `start_time()` | `creation_time_100ns_since_1601()` |
| `ResourceSet` | `ResourceBatch` |
| `with_only_registered` | `with_require_restart_registration` |
| `set_filter(target, action)` | `set_filter(&target, action)` |
| `SessionKey: Display` | explicit `as_str` or `into_string` |
| parse error `Error` | `ParseSessionKeyError` |

Shutdown now consumes the primary session:

```rust,ignore
// 0.1
session.shutdown()?;
session.restart()?;

// 1.0
let pending = session.shutdown();
pending.shutdown_outcome().clone().into_result()?;
let completion = pending.restart();
completion.end()?;
```

Do not use `?` on the native shutdown result before establishing recovery.
The result is retained inside `RestartPending`; inspect it there, but always
restart or explicitly call `leave_stopped`.

`JoinedSession::affected_applications` was removed. The primary installer
owns reporting and workflow control; secondary installers only add resources.

The public API is now present on non-Windows targets. Replace target-gated type
imports with normal imports and handle `ErrorKind::UnsupportedPlatform` at
the point where an OS operation is requested.
