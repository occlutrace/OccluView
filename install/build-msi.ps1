[CmdletBinding()]
param(
    [ValidateSet("debug", "release")]
    [string]$Configuration = "release",
    [string]$Target = "x86_64-pc-windows-msvc",
    [string]$Version = "",
    [string]$OutputDir = "",
    [switch]$SkipBuild,
    [ValidateSet("auto", "none", "certstore", "pfx")]
    [string]$SignMode = "auto",
    [string]$TimestampUrl = ""
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$cargoToml = Join-Path $repoRoot "Cargo.toml"
$wxsPath = Join-Path $PSScriptRoot "occluview.wxs"

if (-not (Test-Path $wxsPath)) {
    throw "Missing WiX source: $wxsPath"
}

function Assert-MsiProductVersion {
    param([Parameter(Mandatory = $true)][string]$Value)

    if ($Value -notmatch '^\d{1,3}\.\d{1,3}\.\d{1,5}$') {
        throw "MSI ProductVersion must be exactly X.Y.Z numeric, got '$Value'."
    }
    $parsed = [version]$Value
    if ($parsed.Major -gt 255 -or $parsed.Minor -gt 255 -or $parsed.Build -gt 65535) {
        throw "MSI ProductVersion '$Value' exceeds Windows Installer version bounds."
    }
    return $Value
}

function Test-HasText {
    param([AllowNull()][string]$Value)

    return -not [string]::IsNullOrWhiteSpace($Value)
}

function Find-SignTool {
    $command = Get-Command signtool.exe -ErrorAction SilentlyContinue
    if ($null -ne $command) {
        return $command.Source
    }

    $kitRoots = @(
        "${env:ProgramFiles(x86)}\Windows Kits\11\bin",
        "${env:ProgramFiles(x86)}\Windows Kits\10\bin"
    )

    foreach ($kitRoot in $kitRoots) {
        if (-not (Test-HasText $kitRoot) -or -not (Test-Path $kitRoot)) {
            continue
        }

        $candidate = Get-ChildItem `
            -Path (Join-Path $kitRoot "*\x64\signtool.exe") `
            -ErrorAction SilentlyContinue |
            Sort-Object FullName -Descending |
            Select-Object -First 1
        if ($null -ne $candidate) {
            return $candidate.FullName
        }
    }

    throw "signtool.exe not found. Install the Windows SDK or put signtool.exe on PATH."
}

function Resolve-SigningMode {
    param([Parameter(Mandatory = $true)][string]$RequestedMode)

    switch ($RequestedMode) {
        "none" {
            return "none"
        }
        "certstore" {
            if (-not (Test-HasText $env:OCCLUVIEW_SIGN_CERT_SHA1)) {
                throw "SignMode certstore requires OCCLUVIEW_SIGN_CERT_SHA1."
            }
            return "certstore"
        }
        "pfx" {
            if (-not (Test-HasText $env:OCCLUVIEW_SIGN_PFX_PATH)) {
                throw "SignMode pfx requires OCCLUVIEW_SIGN_PFX_PATH."
            }
            return "pfx"
        }
        "auto" {
            if (Test-HasText $env:OCCLUVIEW_SIGN_PFX_PATH) {
                return "pfx"
            }
            if (Test-HasText $env:OCCLUVIEW_SIGN_CERT_SHA1) {
                return "certstore"
            }
            return "none"
        }
    }
}

function Sign-WindowsArtifact {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][ValidateSet("none", "certstore", "pfx")][string]$Mode,
        [Parameter(Mandatory = $true)][string]$TimestampUrl
    )

    if ($Mode -eq "none") {
        return
    }
    if (-not (Test-Path $Path)) {
        throw "Signing input missing: $Path"
    }

    $existingSignature = Get-AuthenticodeSignature -FilePath $Path
    if ($existingSignature.Status -eq "Valid") {
        Write-Host "Already signed: $Path"
        return
    }

    $signTool = Find-SignTool
    $signArgs = @("sign", "/fd", "SHA256", "/td", "SHA256")
    if (Test-HasText $TimestampUrl) {
        $signArgs += @("/tr", $TimestampUrl)
    }

    switch ($Mode) {
        "certstore" {
            $thumbprint = $env:OCCLUVIEW_SIGN_CERT_SHA1
            if (-not (Test-HasText $thumbprint)) {
                throw "OCCLUVIEW_SIGN_CERT_SHA1 is required for certstore signing."
            }
            $signArgs += @("/sha1", $thumbprint)
        }
        "pfx" {
            $pfxPath = $env:OCCLUVIEW_SIGN_PFX_PATH
            if (-not (Test-HasText $pfxPath) -or -not (Test-Path $pfxPath)) {
                throw "OCCLUVIEW_SIGN_PFX_PATH does not point to an existing PFX file."
            }
            $signArgs += @("/f", $pfxPath)
            if (Test-HasText $env:OCCLUVIEW_SIGN_PFX_PASSWORD) {
                $signArgs += @("/p", $env:OCCLUVIEW_SIGN_PFX_PASSWORD)
            }
        }
    }

    $signArgs += $Path
    & $signTool @signArgs
    if ($LASTEXITCODE -ne 0) {
        throw "signtool.exe failed for $Path"
    }

    $signature = Get-AuthenticodeSignature -FilePath $Path
    if ($signature.Status -ne "Valid") {
        throw "Authenticode signature for $Path is $($signature.Status): $($signature.StatusMessage)"
    }
    Write-Host "Signed: $Path"
}

if ([string]::IsNullOrWhiteSpace($Version)) {
    $cargoText = Get-Content $cargoToml -Raw
    $match = [regex]::Match($cargoText, '(?s)\[workspace\.package\].*?version\s*=\s*"([^"]+)"')
    if (-not $match.Success) {
        throw "Could not locate [workspace.package].version in $cargoToml"
    }
    $Version = $match.Groups[1].Value
}
$Version = Assert-MsiProductVersion $Version
if ([string]::IsNullOrWhiteSpace($TimestampUrl)) {
    $TimestampUrl = $env:OCCLUVIEW_SIGN_TIMESTAMP_URL
}
if ([string]::IsNullOrWhiteSpace($TimestampUrl)) {
    $TimestampUrl = "http://timestamp.digicert.com"
}

$profileDir = if ($Configuration -eq "release") { "release" } else { "debug" }
$shellProfileDir = if ($Configuration -eq "release") { "release-unwind" } else { "debug" }
$buildDir = Join-Path $repoRoot (Join-Path "target\$Target" $profileDir)
$shellBuildDir = Join-Path $repoRoot (Join-Path "target\$Target" $shellProfileDir)
if ([string]::IsNullOrWhiteSpace($OutputDir)) {
    $OutputDir = Join-Path $repoRoot "dist"
}
$outputDir = $OutputDir
$outputName = "OccluView-$Version-$Target"
$wixObj = Join-Path $outputDir "occluview.wixobj"
$msiPath = Join-Path $outputDir "$outputName.msi"

if (-not $SkipBuild) {
    $cargoArgs = @(
        "build",
        "--locked",
        "-p", "occluview-app",
        "--target", $Target
    )
    # The shell DLL builds in its own unwind profile (see Cargo.toml): a
    # panicking cdylib under panic=abort would kill Explorer's dllhost and
    # blank every thumbnail in the folder.
    $shellCargoArgs = @(
        "build",
        "--locked",
        "-p", "occluview-shell",
        "--target", $Target
    )
    if ($Configuration -eq "release") {
        $cargoArgs += "--release"
        $shellCargoArgs += @("--profile", "release-unwind")
    }
    if (Test-HasText $env:OCCLUVIEW_HPS_EMBEDDED_KEY) {
        Write-Host "Private HPS key embedding enabled for this build."
        # The app and Explorer shell DLL both need the key to decode encrypted
        # HPS scans for the viewer, thumbnails, and the preview pane.
        $cargoArgs += @("--features", "occluview-formats/private-hps-key")
        $shellCargoArgs += @("--features", "occluview-formats/private-hps-key")
    }

    $previousEncodedRustFlags = [Environment]::GetEnvironmentVariable("CARGO_ENCODED_RUSTFLAGS")
    if ($Configuration -eq "release") {
        $separator = [string][char]0x1f
        $releaseRustFlags = @("--remap-path-prefix=$repoRoot=occluview")
        $normalizedRepoRoot = $repoRoot.Replace("\", "/")
        if ($normalizedRepoRoot -ne $repoRoot) {
            $releaseRustFlags += "--remap-path-prefix=$normalizedRepoRoot=occluview"
        }
        $encodedReleaseRustFlags = $releaseRustFlags -join $separator
        if (Test-HasText $previousEncodedRustFlags) {
            $env:CARGO_ENCODED_RUSTFLAGS = "$previousEncodedRustFlags$separator$encodedReleaseRustFlags"
        } else {
            $env:CARGO_ENCODED_RUSTFLAGS = $encodedReleaseRustFlags
        }
    }
    try {
        & cargo @cargoArgs
        if ($LASTEXITCODE -ne 0) {
            throw "cargo build failed"
        }
        & cargo @shellCargoArgs
        if ($LASTEXITCODE -ne 0) {
            throw "cargo shell build failed"
        }
    } finally {
        if ($null -eq $previousEncodedRustFlags) {
            Remove-Item Env:CARGO_ENCODED_RUSTFLAGS -ErrorAction SilentlyContinue
        } else {
            $env:CARGO_ENCODED_RUSTFLAGS = $previousEncodedRustFlags
        }
    }
}

# The wxs harvests both artifacts from one BuildDir: stage the unwind-profile
# DLL next to the exe so the existing -dBuildDir contract stays intact.
# (-SkipBuild reuses a previously staged DLL, so the copy is conditional.)
$shellDllSource = Join-Path $shellBuildDir "occluview_shell.dll"
if (Test-Path $shellDllSource) {
    Copy-Item $shellDllSource (Join-Path $buildDir "occluview_shell.dll") -Force
}
$required = @(
    (Join-Path $buildDir "occluview.exe"),
    (Join-Path $buildDir "occluview_shell.dll")
)
foreach ($path in $required) {
    if (-not (Test-Path $path)) {
        throw "Required build artifact missing: $path"
    }
}

$resolvedSignMode = Resolve-SigningMode $SignMode
if ($resolvedSignMode -eq "none") {
    Write-Host "Signing disabled: no signing certificate configured."
} else {
    foreach ($path in $required) {
        Sign-WindowsArtifact -Path $path -Mode $resolvedSignMode -TimestampUrl $TimestampUrl
    }
}

$candle = Get-Command candle.exe -ErrorAction SilentlyContinue
$light = Get-Command light.exe -ErrorAction SilentlyContinue
if ($null -eq $candle -or $null -eq $light) {
    throw "WiX Toolset v3 not found on PATH. Install candle.exe/light.exe first."
}

New-Item -ItemType Directory -Path $outputDir -Force | Out-Null

$candleArgs = @(
    "-nologo",
    "-arch", "x64",
    "-ext", "WixUIExtension",
    "-dBuildDir=$buildDir",
    "-dProductVersion=$Version",
    "-out", $wixObj,
    $wxsPath
)
& $candle.Source @candleArgs
if ($LASTEXITCODE -ne 0) {
    throw "WiX candle.exe failed"
}

$lightArgs = @(
    "-nologo",
    "-ext", "WixUIExtension",
    "-cultures:en-us",
    "-out", $msiPath,
    $wixObj
)
& $light.Source @lightArgs
if ($LASTEXITCODE -ne 0) {
    throw "WiX light.exe failed"
}

Sign-WindowsArtifact -Path $msiPath -Mode $resolvedSignMode -TimestampUrl $TimestampUrl

Write-Host "Built MSI: $msiPath"
