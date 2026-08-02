$ErrorActionPreference = "Stop"
$script = Join-Path $PSScriptRoot "release-metadata.ps1"
$metadata = cargo metadata --no-deps --format-version 1 --locked | ConvertFrom-Json -Depth 100
$manifest = (Resolve-Path "Cargo.toml").Path
$package = $metadata.packages |
    Where-Object { [IO.Path]::GetFullPath($_.manifest_path) -eq $manifest } |
    Select-Object -First 1
$version = $package.version
$testTemp = if ([string]::IsNullOrEmpty($env:RUNNER_TEMP)) {
    [IO.Path]::GetTempPath()
} else {
    $env:RUNNER_TEMP
}
$datedChangelog = Join-Path $testTemp "restart-manager-dated-changelog.md"
$undatedChangelog = Join-Path $testTemp "restart-manager-undated-changelog.md"
$invalidDateChangelog = Join-Path $testTemp "restart-manager-invalid-date-changelog.md"
$lockMismatch = Join-Path $testTemp "restart-manager-lock-mismatch-$([guid]::NewGuid().ToString('N'))"

function Assert-Fails {
    param(
        [scriptblock] $Operation,
        [string] $MessagePattern
    )

    try {
        & $Operation
    } catch {
        if ($_.Exception.Message -notmatch $MessagePattern) {
            throw "unexpected validation failure: $($_.Exception.Message)"
        }
        return
    }
    throw "validation unexpectedly succeeded; expected '$MessagePattern'"
}

try {
    "## [$version] - 2026-08-02" | Set-Content -Encoding utf8 $datedChangelog
    "## [$version]" | Set-Content -Encoding utf8 $undatedChangelog
    "## [$version] - 2026-99-99" | Set-Content -Encoding utf8 $invalidDateChangelog
    New-Item -ItemType Directory -Path $lockMismatch | Out-Null
    @"
[package]
name = "release-metadata-fixture"
version = "0.2.0"
edition = "2024"

[lib]
path = "lib.rs"

[workspace]
"@ | Set-Content -Encoding utf8 (Join-Path $lockMismatch "Cargo.toml")
    "" | Set-Content -Encoding utf8 (Join-Path $lockMismatch "lib.rs")
    @"
version = 4

[[package]]
name = "release-metadata-fixture"
version = "0.1.0"
"@ | Set-Content -Encoding utf8 (Join-Path $lockMismatch "Cargo.lock")

    & $script -GitHubOutput ""
    & $script -ChangelogPath $datedChangelog -ExpectedTag "v$version" `
        -RequireRelease -RequireAnnotatedTag -TagObjectType tag -GitHubOutput ""

    Assert-Fails {
        & $script -ChangelogPath $datedChangelog -ExpectedTag "not-v$version" `
            -RequireRelease -TagObjectType tag -GitHubOutput ""
    } "does not match"
    Assert-Fails {
        & $script -ChangelogPath $undatedChangelog -ExpectedTag "v$version" `
            -RequireRelease -TagObjectType tag -GitHubOutput ""
    } "dated"
    Assert-Fails {
        & $script -ChangelogPath $invalidDateChangelog -ExpectedTag "v$version" `
            -RequireRelease -TagObjectType tag -GitHubOutput ""
    } "invalid"
    Assert-Fails {
        & $script -ChangelogPath $datedChangelog -ExpectedTag "v$version" `
            -RequireRelease -RequireAnnotatedTag -TagObjectType commit -GitHubOutput ""
    } "must be annotated"
    Assert-Fails {
        & $script -ManifestPath (Join-Path $lockMismatch "Cargo.toml") `
            -ChangelogPath $datedChangelog -GitHubOutput ""
    } "Cargo.lock does not contain"
} finally {
    Remove-Item -LiteralPath $datedChangelog, $undatedChangelog, $invalidDateChangelog `
        -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $lockMismatch -Recurse -Force -ErrorAction SilentlyContinue
}
