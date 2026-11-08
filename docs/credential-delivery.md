---
doppler_automation_app: release_plz
---

# Credential delivery

The release App credentials use Doppler's [native GitHub synchronization](https://docs.doppler.com/docs/github-actions).
The [delivery decision](adr/sync-release-app-credentials-to-the-protected-environment.md) governs the cutover.
Canonical values remain in Doppler, and a purpose-specific config references the automation fields.
The [manifest](../automation/release-app-sync.toml) identifies the repository, environment, and field mapping.

The release-plz environment admits main jobs and prevents administrator bypass.
RELEASE_APP_CLIENT_ID references GITHUB_APP_CLIENT_ID, and RELEASE_APP_PRIVATE_KEY references GITHUB_APP_PRIVATE_KEY.
The workflow passes each named secret only to the App token action.
Tokens target the current repository and request the consumer's permissions.
The App private key retains authority across every installation of the shared App.
GitHub stores copies, and protected main and environment policies replace Doppler workflow and event claims.

The manual **Verify credential access** workflow checks required fields and creates a repository token with contents read access only.
The action revokes the token during cleanup.
Run the workflow from reviewed main after merging.
Keep the existing Doppler identity until replacement authentication succeeds and no consumer needs the identity.

Rotate canonical values in Doppler and verify synchronization and authentication before retiring the previous key.
Keep values and authentication state out of the repository and logs.

The independent crates.io publisher uses `release.yml` and the protected `crates-io` environment.
Confirm the existing package's exact Trusted Publisher binding before its next authorized publication.
The workflow retains its default `publish: false` and accepts no fixed registry key.
A missing package or publisher registration requires its own owner-approved setup.
Credential migration creates no release or tag.
