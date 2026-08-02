## Summary

Describe what changed and why.

## Validation

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
- [ ] `cargo test --workspace --all-targets --all-features --locked -- --test-threads=1`
- [ ] Relevant Windows E2E, portable, documentation, and package checks

## Compatibility and safety

- [ ] Public API snapshots and CHANGELOG are updated when applicable
- [ ] New unsafe code is confined to `src/sys.rs` and its invariants are documented
- [ ] Native tests affect only dedicated fixture processes and temporary files
- [ ] Logs and examples contain no session keys, private paths, or restart arguments
