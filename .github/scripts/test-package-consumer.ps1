$ErrorActionPreference = "Stop"
$helper = Join-Path $PSScriptRoot "package-consumer.ps1"
$metadata = & (Join-Path $PSScriptRoot "release-metadata.ps1") -GitHubOutput ""
$archive = Join-Path "target/package" $metadata.archive_name
$testTemp = if ([string]::IsNullOrEmpty($env:RUNNER_TEMP)) {
    [IO.Path]::GetTempPath()
} else {
    $env:RUNNER_TEMP
}
$testId = [guid]::NewGuid().ToString("N")
$checksum = Join-Path $testTemp "package-consumer-$testId.sha256"
$badChecksum = Join-Path $testTemp "package-consumer-$testId-bad.sha256"
$malformedChecksum = Join-Path $testTemp "package-consumer-$testId-malformed.sha256"

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
    # The test is also useful against an intentionally dirty local worktree.
    # CI and the release workflow package clean checkouts without this flag.
    cargo package --locked --allow-dirty
    if ($LASTEXITCODE -ne 0) {
        throw "cargo package --locked --allow-dirty failed"
    }

    $hash = (Get-FileHash $archive -Algorithm SHA256).Hash.ToLowerInvariant()
    "$hash  $($metadata.archive_name)" | Set-Content -Encoding ascii $checksum
    "$('0' * 64)  $($metadata.archive_name)" | Set-Content -Encoding ascii $badChecksum
    "not-a-checksum" | Set-Content -Encoding ascii $malformedChecksum

    & $helper -PackageName $metadata.package_name `
        -PackageVersion $metadata.package_version `
        -ArchivePath $archive `
        -ExpectedChecksumPath $checksum

    Assert-Fails {
        & $helper -PackageName $metadata.package_name `
            -PackageVersion $metadata.package_version `
            -ArchivePath $archive `
            -ExpectedChecksumPath $badChecksum
    } "digest mismatch"
    Assert-Fails {
        & $helper -PackageName $metadata.package_name `
            -PackageVersion $metadata.package_version `
            -ArchivePath $archive `
            -ExpectedChecksumPath $malformedChecksum
    } "sha256sum format"
    Assert-Fails {
        & $helper -PackageName $metadata.package_name `
            -PackageVersion $metadata.package_version `
            -Registry `
            -ExpectedChecksumPath $checksum `
            -RegistryDownloadUri "http://127.0.0.1:1/unavailable.crate" `
            -RegistryAttempts 1 `
            -RetryDelaySeconds 0
    } "unavailable after 1 attempts"
} finally {
    Remove-Item -LiteralPath $checksum, $badChecksum, $malformedChecksum `
        -ErrorAction SilentlyContinue
}
