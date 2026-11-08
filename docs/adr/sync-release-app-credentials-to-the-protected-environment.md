---
date: 2026-11-09
status: proposed
---

# Sync release App credentials to the protected environment

## Context

The release App requires stored credentials.
Doppler's [identity limit](https://docs.doppler.com/docs/platform-limits) constrains fleet growth.
The release-plz environment admits main and prevents administrator bypass.

## Decision

Use [native GitHub synchronization](https://docs.doppler.com/docs/github-actions) from a purpose-specific Doppler config into the existing environment.
Preserve its branch policy and independent publication approval.
Pass named secrets only to the App token action.
Mint tokens for the current repository and consumer permissions.
Keep registry and attestation OIDC.

## Alternatives

Doppler OIDC avoids GitHub copies and preserves workflow and event claims, but consumes identity capacity.
A service token adds another stored credential.

## Consequences

GitHub stores credential copies.
Protected main and environment policies replace Doppler claims.
The shared App key retains authority across its installations.
Reconsider when GitHub supports workload identity for the App.

## Confirmation

Verify the exact synchronization target, initial synchronization, and manual authentication from reviewed main before identity retirement.
Migration creates no release or tag.
