[CmdletBinding(DefaultParameterSetName = "Archive")]
param(
    [Parameter(Mandatory)]
    [ValidateNotNullOrEmpty()]
    [string] $PackageName,

    [Parameter(Mandatory)]
    [ValidateNotNullOrEmpty()]
    [string] $PackageVersion,

    [Parameter(Mandatory, ParameterSetName = "Archive")]
    [ValidateNotNullOrEmpty()]
    [string] $ArchivePath,

    [Parameter(Mandatory, ParameterSetName = "Registry")]
    [switch] $Registry,

    [ValidateNotNullOrEmpty()]
    [string] $ExpectedChecksumPath,

    [Parameter(ParameterSetName = "Registry")]
    [ValidateRange(1, 100)]
    [int] $RegistryAttempts = 30,

    [Parameter(ParameterSetName = "Registry")]
    [ValidateRange(0, 300)]
    [int] $RetryDelaySeconds = 10,

    [Parameter(ParameterSetName = "Registry")]
    [ValidateNotNullOrEmpty()]
    [string] $RegistryDownloadUri = ""
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Invoke-Cargo {
    param([Parameter(Mandatory)][string[]] $Arguments)

    & cargo @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "cargo $($Arguments -join ' ') failed with exit code $LASTEXITCODE"
    }
}

function Assert-ArchiveDigest {
    param(
        [Parameter(Mandatory)][string] $Path,
        [Parameter(Mandatory)][string] $ChecksumPath
    )

    $checksum = (Get-Content -Raw $ChecksumPath).Trim()
    $parsed = [regex]::Match(
        $checksum,
        '\A(?<hash>[0-9a-fA-F]{64}) [ *](?<name>[^\r\n]+)\z'
    )
    if (-not $parsed.Success) {
        throw "checksum must use sha256sum format: <64 hex characters><two spaces><filename>"
    }

    $archiveName = Split-Path -Leaf $Path
    if ($parsed.Groups["name"].Value -ne $archiveName) {
        throw "checksum names '$($parsed.Groups["name"].Value)', expected '$archiveName'"
    }

    $actual = (Get-FileHash $Path -Algorithm SHA256).Hash.ToLowerInvariant()
    $expected = $parsed.Groups["hash"].Value.ToLowerInvariant()
    if ($actual -ne $expected) {
        throw "package archive digest mismatch: expected $expected, got $actual"
    }
}

function New-Consumer {
    param(
        [Parameter(Mandatory)][string] $Root,
        [Parameter(Mandatory)][ValidateSet("default", "tokio")][string] $Variant,
        [Parameter(Mandatory)][string] $Dependency
    )

    $consumer = Join-Path $Root "consumer-$Variant"
    $source = Join-Path $consumer "src"
    New-Item -ItemType Directory -Path $source -Force | Out-Null
    $featureClause = if ($Variant -eq "tokio") { ', features = ["tokio"]' } else { "" }
    @"
[package]
name = "package-consumer-$Variant"
version = "0.0.0"
edition = "2024"

[dependencies]
$PackageName = { $Dependency$featureClause }
"@ | Set-Content -Encoding utf8 (Join-Path $consumer "Cargo.toml")

    $crateIdentifier = $PackageName.Replace("-", "_")
    @"
fn main() -> Result<(), $crateIdentifier::Error> {
    let mut session = $crateIdentifier::RestartSession::new()?;
    session.register_files([r"C:\product\component.dll"])?;

    let applications = session.affected_applications()?;
    eprintln!("reboot reasons: {:?}", applications.reboot_reasons());
    for application in &applications {
        eprintln!(
            "{} (restartable: {})",
            application.display_name().to_string_lossy(),
            application.is_restartable(),
        );
    }

    session.end()
}
"@ | Set-Content -Encoding utf8 (Join-Path $source "main.rs")

    $consumer
}

function Test-Consumers {
    param(
        [Parameter(Mandatory)][string] $Root,
        [Parameter(Mandatory)][string] $Dependency
    )

    foreach ($variant in @("default", "tokio")) {
        $consumer = New-Consumer -Root $Root -Variant $variant -Dependency $Dependency
        Invoke-Cargo -Arguments @(
            "generate-lockfile",
            "--manifest-path", (Join-Path $consumer "Cargo.toml")
        )
        Invoke-Cargo -Arguments @(
            "check",
            "--manifest-path", (Join-Path $consumer "Cargo.toml"),
            "--locked"
        )
    }
}

$resolvedArchiveInput = if ($PSCmdlet.ParameterSetName -eq "Archive") {
    (Resolve-Path $ArchivePath).Path
} else {
    ""
}
$resolvedChecksumInput = if ([string]::IsNullOrEmpty($ExpectedChecksumPath)) {
    ""
} else {
    (Resolve-Path $ExpectedChecksumPath).Path
}
$temporaryBase = if ([string]::IsNullOrEmpty($env:RUNNER_TEMP)) {
    [IO.Path]::GetTempPath()
} else {
    $env:RUNNER_TEMP
}
$workRoot = Join-Path $temporaryBase "package-consumer-$([guid]::NewGuid().ToString('N'))"
New-Item -ItemType Directory -Path $workRoot | Out-Null

$previousCargoHome = [Environment]::GetEnvironmentVariable("CARGO_HOME", "Process")
$locationPushed = $false
try {
    Push-Location $workRoot
    $locationPushed = $true

    if ($PSCmdlet.ParameterSetName -eq "Archive") {
        if (-not [string]::IsNullOrEmpty($resolvedChecksumInput)) {
            Assert-ArchiveDigest -Path $resolvedArchiveInput `
                -ChecksumPath $resolvedChecksumInput
        }

        $unpack = Join-Path $workRoot "unpacked"
        New-Item -ItemType Directory -Path $unpack | Out-Null
        & tar -xf $resolvedArchiveInput -C $unpack
        if ($LASTEXITCODE -ne 0) {
            throw "failed to unpack '$resolvedArchiveInput'"
        }

        $source = Join-Path $unpack "$PackageName-$PackageVersion"
        if (-not (Test-Path -LiteralPath (Join-Path $source "Cargo.toml") -PathType Leaf)) {
            throw "archive does not contain '$PackageName-$PackageVersion/Cargo.toml'"
        }
        $portableSource = ([IO.Path]::GetFullPath($source)).Replace('\', '/')
        if ($portableSource.Contains('"')) {
            throw "package source path cannot be represented safely in Cargo.toml"
        }
        Test-Consumers -Root $workRoot -Dependency "path = `"$portableSource`""
        return
    }

    if ([string]::IsNullOrEmpty($resolvedChecksumInput)) {
        throw "-ExpectedChecksumPath is required for registry verification"
    }

    $env:CARGO_HOME = Join-Path $workRoot "cargo-home"
    New-Item -ItemType Directory -Path $env:CARGO_HOME | Out-Null
    $archiveName = "$PackageName-$PackageVersion.crate"
    $downloadedArchive = Join-Path $workRoot $archiveName
    $downloadUri = if ([string]::IsNullOrEmpty($RegistryDownloadUri)) {
        $escapedName = [uri]::EscapeDataString($PackageName)
        $escapedVersion = [uri]::EscapeDataString($PackageVersion)
        "https://crates.io/api/v1/crates/$escapedName/$escapedVersion/download"
    } else {
        $RegistryDownloadUri
    }

    $downloaded = $false
    $lastDownloadError = ""
    for ($attempt = 1; $attempt -le $RegistryAttempts; $attempt++) {
        try {
            Invoke-WebRequest -Uri $downloadUri -OutFile $downloadedArchive `
                -TimeoutSec 30 `
                -Headers @{ "User-Agent" = "restart-manager-release-verifier" }
            $downloaded = $true
            break
        } catch {
            $lastDownloadError = $_.Exception.Message
            if ($attempt -lt $RegistryAttempts -and $RetryDelaySeconds -gt 0) {
                Start-Sleep -Seconds $RetryDelaySeconds
            }
        }
    }
    if (-not $downloaded) {
        throw "registry archive was unavailable after $RegistryAttempts attempts: $lastDownloadError"
    }
    Assert-ArchiveDigest -Path $downloadedArchive `
        -ChecksumPath $resolvedChecksumInput

    $dependency = "version = `"=$PackageVersion`""
    $lastConsumerError = ""
    for ($attempt = 1; $attempt -le $RegistryAttempts; $attempt++) {
        try {
            Test-Consumers -Root $workRoot -Dependency $dependency
            return
        } catch {
            $lastConsumerError = $_.Exception.Message
            if ($attempt -lt $RegistryAttempts -and $RetryDelaySeconds -gt 0) {
                Start-Sleep -Seconds $RetryDelaySeconds
            }
        }
    }
    throw "registry consumers were unavailable after $RegistryAttempts attempts: $lastConsumerError"
} finally {
    if ($locationPushed) {
        Pop-Location
    }
    if ($null -eq $previousCargoHome) {
        Remove-Item Env:CARGO_HOME -ErrorAction SilentlyContinue
    } else {
        $env:CARGO_HOME = $previousCargoHome
    }
    Remove-Item -LiteralPath $workRoot -Recurse -Force -ErrorAction SilentlyContinue
}
