#!/usr/bin/env bash
set -euo pipefail

policy=.github/scripts/dependabot-auto-merge-policy.jq

update() {
  jq -cn \
    --arg ecosystem "$1" \
    --arg update_type "$2" \
    --arg previous "$3" \
    --argjson maintainer_changes "$4" \
    --arg dependency_name "${5:-example/dependency}" \
    '[{
      dependencyName: $dependency_name,
      packageEcosystem: $ecosystem,
      updateType: $update_type,
      prevVersion: $previous,
      maintainerChanges: $maintainer_changes
    }]'
}

expect() {
  local expected=$1
  local description=$2
  local input=$3
  local actual=false

  if jq -e -f "$policy" <<<"$input" > /dev/null; then
    actual=true
  fi
  if [[ "$actual" != "$expected" ]]; then
    echo "$description: expected $expected, got $actual" >&2
    exit 1
  fi
}

cargo_patch=$(update cargo version-update:semver-patch 0.1.0 false)
cargo_stable_minor=$(update cargo version-update:semver-minor 1.2.3 false)
actions_minor=$(update github-actions version-update:semver-minor v6.0.0 false)

expect true "Cargo 0.x patch" "$cargo_patch"
expect true "Cargo stable minor" "$cargo_stable_minor"
expect true "GitHub Actions minor" "$actions_minor"
expect true "compatible group" "$(jq -cn --argjson a "$cargo_patch" --argjson b "$actions_minor" '$a + $b')"

expect false "Cargo 0.x minor" "$(update cargo version-update:semver-minor 0.1.0 false)"
expect false "Cargo 0.0.x patch" "$(update cargo version-update:semver-patch 0.0.3 false)"
expect false "GitHub Actions 0.x patch" "$(
  update github-actions version-update:semver-patch v0.5.1 false
)"
expect false "major update" "$(update cargo version-update:semver-major 1.2.3 false)"
expect false "same-version or digest update" "$(update github-actions '' v6.0.0 false)"
expect false "maintainer changes" "$(update cargo version-update:semver-patch 1.2.3 true)"
expect false "unparsable Cargo version" "$(update cargo version-update:semver-minor main false)"
expect false "unknown ecosystem" "$(update npm version-update:semver-patch 1.2.3 false)"
expect false "credential-bearing Action" "$(
  update github-actions version-update:semver-patch v3.2.0 false actions/create-github-app-token
)"
expect false "auto-merge metadata Action" "$(
  update github-actions version-update:semver-patch v3.1.0 false dependabot/fetch-metadata
)"
expect false "empty metadata" '[]'
expect false "group containing a major" "$(
  jq -cn \
    --argjson a "$cargo_patch" \
    --argjson b "$(update cargo version-update:semver-major 1.2.3 false)" \
    '$a + $b'
)"
