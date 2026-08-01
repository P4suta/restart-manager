# Contributing

Use Rust 1.88 or newer. Keep normal dependencies limited to `windows-sys`,
`thiserror`, `bitflags`, and optional `tokio` unless an accepted design
record changes that policy.

Before submitting a change, run:

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-targets --all-features --locked -- --test-threads=1
cargo test --doc --all-features --locked
cargo xtask e2e
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --locked --no-deps
cargo package --locked
```

Native tests and E2E scenarios must act only on dedicated fixture processes and
temporary files. Never point a shutdown test at an editor, shell, build tool,
or unrelated user process.

Public API changes require updated default/all-feature snapshots, semver review,
CHANGELOG and migration notes, and a design record when lifecycle or unsafe
invariants change.
