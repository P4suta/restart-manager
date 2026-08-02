# Initial 0.1.0 release runbook

Version 0.1.0 has not been published. Release-plz prepares a version and
CHANGELOG pull request, but tag creation and crates.io publication remain
deliberate operator actions. The release pull request is intentionally left for
human review; merging it does not create a tag, GitHub Release, or crates.io
publication.

## Release pull-request automation

The `Release-plz release PR` workflow uses the `release-plz` GitHub environment,
which accepts deployments only from `main`. It is a successful no-op until a
repository-only GitHub App is installed. Give the App only these repository
permissions:

- Contents: read and write.
- Pull requests: read and write.

In Settings, Environments, `release-plz`, store its client ID as the
`RELEASE_PLZ_APP_CLIENT_ID` environment secret and its private key as
`RELEASE_PLZ_APP_PRIVATE_KEY`. The workflow activates automatically when both
exist; no repository variable is required. With neither secret it is a
successful no-op, while a partial credential pair is rejected as a
configuration error. The App token is scoped again to this repository by the
workflow. Do not substitute a broad personal access token.

Release-plz updates `Cargo.toml`, `Cargo.lock`, and `CHANGELOG.md` on its release
branch. It cannot publish, create or move a tag, or create a GitHub Release.
For the initial 0.1.0 pull request, reconcile generated entries with the
hand-written `Unreleased` notes and remove duplicate headings before merging.

Before tagging:

1. Merge the reviewed release-plz pull request and confirm `Cargo.toml` and
   `Cargo.lock` both describe 0.1.0.
2. Confirm the CHANGELOG contains `## [0.1.0] - YYYY-MM-DD` and review the
   complete first-release notes. Keep `## [Unreleased]` for future changes.
3. Run every locked command from CONTRIBUTING, native E2E, coverage, default
   and all-feature API snapshots, semver review, and actionlint.
4. Run `cargo package --locked`, inspect the allowlist, unpack the crate, and
   compile a clean default/all-feature consumer.
5. Review the generated SBOM and provenance inputs.
6. Create the annotated `v0.1.0` tag only after the commit is on protected
   `main`.

After the release pull request is merged, the release workflow derives the
crate name, version, archive name, source
directory, and artifact name from Cargo metadata. A publish request is rejected
unless it runs from the matching annotated tag and the CHANGELOG has the dated
version heading. The protected `crates-io` environment must also approve it.

For the first crates.io release, use a scoped token to bootstrap package
ownership. After ownership exists, configure the repository/environment as a
Trusted Publisher, remove the token secret, and enable the trusted-publishing
path. Keep token publishing disabled after that transition.
