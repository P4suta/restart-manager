# Security policy

Security fixes are provided for the latest 1.x release.

Please report vulnerabilities privately through the repository's GitHub
Security Advisory form. Do not open a public issue containing exploit details,
session keys, application restart arguments, or affected executable paths.

The crate treats session keys and application restart arguments as sensitive:
their `Debug` implementations are redacted. Consumers remain responsible for
not logging values returned explicitly by `SessionKey::as_str` or
`SessionKey::into_string`.

Unsafe code is confined to the private Windows system boundary. CI runs strict
Clippy, rustdoc, dependency policy, CodeQL, native tests, portable target
checks, and coverage gates.
