[CmdletBinding()]
param(
    [string] $ManifestPath = "Cargo.toml",
    [string] $ChangelogPath = "CHANGELOG.md",
    [AllowEmptyString()]
    [string] $ExpectedTag = "",
    [switch] $RequireRelease,
    [switch] $RequireAnnotatedTag,
    [ValidateSet("", "commit", "tag")]
    [string] $TagObjectType = "",
    [AllowEmptyString()]
    [string] $GitHubOutput = $env:GITHUB_OUTPUT
)

$ErrorActionPreference = "Stop"

$metadataJson = & cargo metadata --no-deps --format-version 1 --locked --manifest-path $ManifestPath
if ($LASTEXITCODE -ne 0) {
    throw "cargo metadata --locked failed; Cargo.toml and Cargo.lock may disagree"
}
$metadata = $metadataJson | ConvertFrom-Json -Depth 100
$resolvedManifest = (Resolve-Path $ManifestPath).Path
$package = $metadata.packages |
    Where-Object { [IO.Path]::GetFullPath($_.manifest_path) -eq $resolvedManifest } |
    Select-Object -First 1
if ($null -eq $package) {
    throw "Cargo metadata did not contain the package at $resolvedManifest"
}

$crateName = $package.name
$version = $package.version
$lockfilePath = Join-Path $metadata.workspace_root "Cargo.lock"
$lockfile = Get-Content -Raw $lockfilePath
$packageBlocks = [regex]::Matches(
    $lockfile,
    "(?ms)^\[\[package\]\]\r?\n(?<body>.*?)(?=^\[\[package\]\]|\z)"
)
$escapedName = [regex]::Escape($crateName)
$workspaceEntries = @($packageBlocks | Where-Object {
    $_.Groups["body"].Value -match "(?m)^name = `"$escapedName`"\r?$" -and
    $_.Groups["body"].Value -notmatch "(?m)^source = "
})
$lockedVersions = @($workspaceEntries | ForEach-Object {
    [regex]::Match(
        $_.Groups["body"].Value,
        '(?m)^version = "([^"]+)"\r?$'
    ).Groups[1].Value
})
if ($lockedVersions -notcontains $version) {
    $found = if ($lockedVersions.Count -eq 0) { "<missing>" } else { $lockedVersions -join ", " }
    throw "Cargo.lock does not contain workspace package '$crateName' at version $version (found: $found)"
}

$archiveName = "$crateName-$version.crate"
$artifactName = "verified-$crateName-$version"
$sourceDirectory = "$crateName-$version"
$checksumName = "$archiveName.sha256"
$sbomName = "$crateName-$version.spdx.json"
$changelog = Get-Content -Raw $ChangelogPath

if ($RequireRelease) {
    $requiredTag = "v$version"
    if ($ExpectedTag -ne $requiredTag) {
        throw "tag '$ExpectedTag' does not match '$requiredTag' from Cargo metadata"
    }
    $escapedVersion = [regex]::Escape($version)
    $releaseHeading = [regex]::Match(
        $changelog,
        "(?m)^## \[$escapedVersion\] - (?<date>\d{4}-\d{2}-\d{2})\r?$"
    )
    if (-not $releaseHeading.Success) {
        throw "CHANGELOG must contain a dated '## [$version] - YYYY-MM-DD' heading"
    }
    $parsedDate = [datetime]::MinValue
    if (-not [datetime]::TryParseExact(
        $releaseHeading.Groups["date"].Value,
        "yyyy-MM-dd",
        [Globalization.CultureInfo]::InvariantCulture,
        [Globalization.DateTimeStyles]::None,
        [ref] $parsedDate
    )) {
        throw "CHANGELOG release date '$($releaseHeading.Groups["date"].Value)' is invalid"
    }
} elseif ($changelog -notmatch "(?m)^## \[Unreleased\]\r?$") {
    throw "the unpublished package must keep a top-level '## [Unreleased]' heading"
}

if ($RequireAnnotatedTag) {
    $objectType = $TagObjectType
    if ([string]::IsNullOrEmpty($objectType)) {
        $objectType = (& git cat-file -t "refs/tags/$ExpectedTag").Trim()
        if ($LASTEXITCODE -ne 0) {
            throw "tag '$ExpectedTag' is unavailable to git"
        }
    }
    if ($objectType -ne "tag") {
        throw "tag '$ExpectedTag' must be annotated, but its object type is '$objectType'"
    }
}

$values = [ordered]@{
    package_name = $crateName
    package_version = $version
    archive_name = $archiveName
    archive_path = "target/package/$archiveName"
    source_directory = $sourceDirectory
    artifact_name = $artifactName
    checksum_name = $checksumName
    sbom_name = $sbomName
}

if (-not [string]::IsNullOrEmpty($GitHubOutput)) {
    foreach ($entry in $values.GetEnumerator()) {
        "$($entry.Key)=$($entry.Value)" | Out-File -FilePath $GitHubOutput -Encoding utf8 -Append
    }
}

[PSCustomObject]$values
