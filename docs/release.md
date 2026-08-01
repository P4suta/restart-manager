# 1.x release runbook

The repository, crates.io package, initial push, and first publication are
manual bootstrap steps and are not performed by automation in this repository.

Before tagging:

1. Confirm `Cargo.toml`, `Cargo.lock`, the top CHANGELOG entry, and the tag
   all contain the same version.
2. Run every locked command from CONTRIBUTING, native E2E, coverage, default
   and all-feature API snapshots, and semver checks.
3. Run `cargo package --locked`, inspect the allowlist, unpack the crate, and
   compile a clean default/all-feature consumer.
4. Review the generated SBOM and provenance inputs.
5. Create an annotated `vMAJOR.MINOR.PATCH` tag only after the commit is on
   protected `main`.

The release workflow is deliberately dormant unless its publish input is
enabled and the protected `crates-io` environment approves it.

For the first crates.io release, use a scoped token to bootstrap package
ownership. After ownership exists, configure the repository/environment as a
Trusted Publisher, remove the token secret, and enable the trusted-publishing
job. Keep the manual token job disabled after that transition.
