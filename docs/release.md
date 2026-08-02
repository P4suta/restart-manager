# Release runbook

Release-plz prepares a human-reviewed version and changelog pull request. It
does not publish, tag, or create a GitHub Release. The release workflow uses the
crates.io `.crate` archive as the release artifact and keeps publication behind
the protected `crates-io` environment.

## Prepare and verify

1. Merge the reviewed release PR. Confirm `Cargo.toml`, `Cargo.lock`, and the
   dated `## [version] - YYYY-MM-DD` changelog heading agree.
2. Run the locked checks in [CONTRIBUTING.md](../CONTRIBUTING.md), plus
   `actionlint` and the scripts in `.github/scripts/test-*.ps1`.
3. Create an annotated `v<version>` tag on protected `main` and push it.

The tag run verifies the tag, package metadata, lockfile, and changelog; runs
the Rust checks; and executes `cargo package --locked`. From that one archive it
creates a standard SHA-256 checksum and an SPDX JSON SBOM, then records both a
build-provenance attestation and an SBOM attestation. Anchore's implicit
artifact and release uploads are disabled. The workflow artifact contains the
archive, checksum, SBOM, and the audit-only `package-files.txt`.

Tag/version mismatch, a lightweight tag, an undated changelog entry, or a
packaged default/Tokio consumer failure stops here.

## Publish

Run **Release verification and publishing** manually against the annotated tag
with `publish` enabled. Approve the protected `crates-io` environment only after
the verification job succeeds.

For the first `0.1.0` publication only, disable `trusted_publishing` and provide
a package-scoped `CARGO_REGISTRY_TOKEN` in the environment. After crates.io has
created the package, configure this repository and workflow as a crates.io
Trusted Publisher, remove the token secret, and leave `trusted_publishing`
enabled for every later release.

Cargo cannot upload the already-created archive directly: `cargo publish`
packages the source again. The workflow therefore reproduces and compares the
pre-publish digest, publishes, downloads the immutable archive from crates.io,
and compares its digest with the original. It uses bounded retries while the
registry becomes visible, then checks fresh `=version` default and Tokio
consumers with an isolated `CARGO_HOME`.

Only after those checks pass does the workflow create or reuse a draft GitHub
Release for the existing tag, attach the `.crate`, checksum, and SPDX JSON, and
publish it. A retry reuses matching assets; it refuses to replace any asset with
different bytes. `package-files.txt` remains a workflow artifact and is not a
release asset. See Cargo's [publishing behavior][cargo-publish] and GitHub's
[release guidance][github-releases].

## Verify a published release

Download the three release assets, then run:

```console
sha256sum -c restart-manager-<version>.crate.sha256
gh attestation verify restart-manager-<version>.crate \
  --repo P4suta/restart-manager
gh attestation verify restart-manager-<version>.crate \
  --repo P4suta/restart-manager \
  --predicate-type https://spdx.dev/Document/v2.3
cargo add restart-manager@=<version>
```

The first attestation command verifies SLSA build provenance; the second
verifies the SPDX SBOM predicate. GitHub documents both forms in its
[artifact-attestation guide][github-attestations].

[cargo-publish]: https://doc.rust-lang.org/cargo/reference/publishing.html
[github-attestations]: https://docs.github.com/en/actions/how-tos/secure-your-work/use-artifact-attestations/use-artifact-attestations
[github-releases]: https://docs.github.com/en/repositories/releasing-projects-on-github/managing-releases-in-a-repository
