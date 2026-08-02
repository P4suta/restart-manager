# Initial 0.1.0 release runbook

Version 0.1.0 has not been published. The initial GitHub push, tag creation,
and crates.io publication remain deliberate operator actions; automation in
this repository does not perform them without an explicit publish request.

Before tagging:

1. Confirm `Cargo.toml` and `Cargo.lock` both describe 0.1.0.
2. Replace `## [Unreleased]` in the CHANGELOG with
   `## [0.1.0] - YYYY-MM-DD` and review the complete first-release notes.
3. Run every locked command from CONTRIBUTING, native E2E, coverage, default
   and all-feature API snapshots, semver review, and actionlint.
4. Run `cargo package --locked`, inspect the allowlist, unpack the crate, and
   compile a clean default/all-feature consumer.
5. Review the generated SBOM and provenance inputs.
6. Create the annotated `v0.1.0` tag only after the commit is on protected
   `main`.

The release workflow derives the crate name, version, archive name, source
directory, and artifact name from Cargo metadata. A publish request is rejected
unless it runs from the matching annotated tag and the CHANGELOG has the dated
version heading. The protected `crates-io` environment must also approve it.

For the first crates.io release, use a scoped token to bootstrap package
ownership. After ownership exists, configure the repository/environment as a
Trusted Publisher, remove the token secret, and enable the trusted-publishing
path. Keep token publishing disabled after that transition.
