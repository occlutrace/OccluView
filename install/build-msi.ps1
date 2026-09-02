[CmdletBinding()]
param(
    [ValidateSet("debug", "release", "diagnostic")]
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
# Explorer loads the shell extension independently from occluview.exe. Keep
# the release DLL on the last source revision verified in Explorer while the
# application remains on the current workspace and dependency graph.
$referenceShellRevision = "659725632dffcdf14d62724743f35f1689602bbc"

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

function Set-ReferenceShellPackageVersion {
    param(
        [Parameter(Mandatory = $true)][string]$ReferenceRoot,
        [Parameter(Mandatory = $true)][string]$PackageVersion
    )

    # This only changes the transient worktree's package metadata so Windows
    # Installer sees the reference DLL as the same version as its MSI. The
    # compiled shell code and its pinned dependency graph remain the exact
    # working revision. Keep Cargo.lock synchronized and still build locked.
    $referenceCargoToml = Join-Path $ReferenceRoot "Cargo.toml"
    $referenceCargoLock = Join-Path $ReferenceRoot "Cargo.lock"
    $toml = [IO.File]::ReadAllText($referenceCargoToml)
    $workspaceVersionPattern = '(?ms)(\[workspace\.package\][^\[]*?^version\s*=\s*")[^"]+(")'
    $updatedToml = [regex]::Replace($toml, $workspaceVersionPattern, "`${1}$PackageVersion`${2}", 1)
    if ($updatedToml -eq $toml) {
        throw "Could not set the reference shell package version in $referenceCargoToml"
    }
    [IO.File]::WriteAllText($referenceCargoToml, $updatedToml, [Text.UTF8Encoding]::new($false))

    $lock = [IO.File]::ReadAllText($referenceCargoLock)
    foreach ($package in @(
        "occlu-mesh-edit",
        "occluview-align",
        "occluview-app",
        "occluview-cli",
        "occluview-core",
        "occluview-formats",
        "occluview-hps",
        "occluview-render",
        "occluview-robust-csg",
        "occluview-shell",
        "occluview-thumbnail",
        "occluview-update"
    )) {
        $packagePattern = "(?ms)(\[\[package\]\]\r?\nname = `"$package`"\r?\nversion = `")[^`"]+(`")"
        $packageEntries = [regex]::Matches($lock, $packagePattern)
        if ($packageEntries.Count -ne 1) {
            throw "Expected exactly one Cargo.lock package entry for $package in the reference shell worktree."
        }
        $lock = [regex]::Replace($lock, $packagePattern, "`${1}$PackageVersion`${2}", 1)
    }
    [IO.File]::WriteAllText($referenceCargoLock, $lock, [Text.UTF8Encoding]::new($false))
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

function Find-DumpBin {
    $command = Get-Command dumpbin.exe -ErrorAction SilentlyContinue
    if ($null -ne $command) {
        return $command.Source
    }

    # GitHub's Windows image has the native tools installed, but does not put
    # their Visual Studio directory on PATH for a plain pwsh step. Locate the
    # selected MSVC toolset through the supported Visual Studio Installer
    # entry point instead of relying on a runner-specific PATH mutation.
    $programFilesX86 = [Environment]::GetFolderPath([Environment+SpecialFolder]::ProgramFilesX86)
    if (-not (Test-HasText $programFilesX86)) {
        return $null
    }
    $vswhere = Join-Path $programFilesX86 "Microsoft Visual Studio\Installer\vswhere.exe"
    if (-not (Test-Path $vswhere)) {
        return $null
    }

    $installations = @(& $vswhere -products * -sort -property installationPath)
    if ($LASTEXITCODE -ne 0) {
        return $null
    }
    foreach ($installation in $installations) {
        $installationPath = "$installation".Trim()
        if (-not (Test-HasText $installationPath)) {
            continue
        }
        $versionFile = Join-Path $installationPath "VC\Auxiliary\Build\Microsoft.VCToolsVersion.default.txt"
        if (-not (Test-Path $versionFile)) {
            continue
        }
        $toolsetVersion = (Get-Content $versionFile -Raw).Trim()
        if (-not (Test-HasText $toolsetVersion)) {
            continue
        }
        $candidate = Join-Path $installationPath "VC\Tools\MSVC\$toolsetVersion\bin\Hostx64\x64\dumpbin.exe"
        if (Test-Path $candidate) {
            return $candidate
        }
    }

    return $null
}

function Assert-StaticMsvcRuntime {
    param([Parameter(Mandatory = $true)][string[]]$Paths)

    $dumpbin = Find-DumpBin
    if (-not (Test-HasText $dumpbin)) {
        throw "dumpbin.exe is required to verify that the MSI payload does not depend on the VC++ runtime."
    }

    foreach ($path in $Paths) {
        $imports = & $dumpbin /imports $path 2>&1
        if ($LASTEXITCODE -ne 0) {
            throw "dumpbin.exe could not inspect imports for $path"
        }
        if ($imports -match '(?im)^\s*(VCRUNTIME[0-9_]*\.DLL|MSVCP[0-9_]*\.DLL|UCRTBASE\.DLL|API-MS-WIN-CRT-[A-Z0-9_-]*\.DLL)\s*$') {
            throw "MSI payload $path depends on a separately installed VC++ runtime."
        }
    }
}

function Assert-ManifoldStaticMsvcRuntime {
    param(
        [Parameter(Mandatory = $true)][string]$Target,
        [Parameter(Mandatory = $true)][string]$RepoRoot
    )

    $targetDir = Join-Path $RepoRoot "target\\$Target"
    $caches = @(
        Get-ChildItem -Path $targetDir -Filter "CMakeCache.txt" -Recurse -File -ErrorAction SilentlyContinue |
            Where-Object { $_.FullName -match '[\\/]manifold-csg-sys-[^\\/]+[\\/]out[\\/]build[\\/]CMakeCache\.txt$' }
    )
    if ($caches.Count -eq 0) {
        throw "Manifold did not produce a CMake cache for $Target."
    }

    foreach ($cache in $caches) {
        if (-not (Select-String -Path $cache.FullName -Pattern '^CMAKE_MSVC_RUNTIME_LIBRARY:STRING=MultiThreaded$' -Quiet)) {
            throw "Manifold cache $($cache.FullName) is not configured for the static MSVC runtime."
        }
    }
}

function Get-DiagnosticManifestEntry {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$InstallPath,
        [Parameter(Mandatory = $true)][ValidateSet("binary", "symbol", "helper", "guide")][string]$Kind
    )

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "Diagnostic package input missing: $Path"
    }
    $item = Get-Item -LiteralPath $Path
    $hash = Get-FileHash -Algorithm SHA256 -LiteralPath $Path
    return [ordered]@{
        path = $InstallPath
        kind = $Kind
        bytes = $item.Length
        sha256 = $hash.Hash.ToLowerInvariant()
    }
}

function Write-DiagnosticManifest {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$MsiPath,
        [Parameter(Mandatory = $true)][string]$Version,
        [Parameter(Mandatory = $true)][string]$Target,
        [Parameter(Mandatory = $true)][object[]]$Contents
    )

    $manifest = [ordered]@{
        schema = "occluview-diagnostic-package-v1"
        version = $Version
        target = $Target
        package = [ordered]@{
            file = (Split-Path -Leaf $MsiPath)
            sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $MsiPath).Hash.ToLowerInvariant()
        }
        contents = $Contents
    }
    $json = $manifest | ConvertTo-Json -Depth 4
    [System.IO.File]::WriteAllText(
        $Path,
        $json + [Environment]::NewLine,
        [System.Text.UTF8Encoding]::new($false)
    )
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

$profileDir = switch ($Configuration) {
    "release" { "release" }
    "debug" { "debug" }
    "diagnostic" { "release-diagnostic" }
}
$shellProfileDir = switch ($Configuration) {
    "release" { "release-unwind" }
    "debug" { "debug" }
    "diagnostic" { "release-diagnostic-unwind" }
}
$isDiagnosticPackage = $Configuration -eq "diagnostic"
# A diagnostic MSI is intentionally a non-release artifact. When built from
# the manual workflow it is downloadable to repository readers from that run,
# never published as a GitHub Release asset; package only remapped PDBs and
# safe JSONL helpers, never patient data.
$buildDir = Join-Path $repoRoot (Join-Path "target\$Target" $profileDir)
$shellBuildDir = Join-Path $repoRoot (Join-Path "target\$Target" $shellProfileDir)
$shellDllSource = Join-Path $shellBuildDir "occluview_shell.dll"
if ([string]::IsNullOrWhiteSpace($OutputDir)) {
    $OutputDir = Join-Path $repoRoot "dist"
}
$outputDir = $OutputDir
$outputName = "OccluView-$Version-$Target"
$wixObj = Join-Path $outputDir "occluview.wixobj"
$msiLeafName = if ($isDiagnosticPackage) { "$outputName-diagnostic.msi" } else { "$outputName.msi" }
$msiPath = Join-Path $outputDir $msiLeafName
$diagnosticWixDefine = if ($isDiagnosticPackage) { "-dIncludeDiagnostics=1" } else { "-dIncludeDiagnostics=0" }
$manifestPath = if ($isDiagnosticPackage) {
    Join-Path $outputDir "$outputName-diagnostic.manifest.json"
} else {
    $null
}

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
    if ($isDiagnosticPackage) {
        $cargoArgs += @("--profile", "release-diagnostic")
        $shellCargoArgs += @("--profile", "release-diagnostic-unwind")
    }
    if (Test-HasText $env:OCCLUVIEW_HPS_EMBEDDED_KEY) {
        Write-Host "Private HPS key embedding enabled for this build."
        # The app and Explorer shell DLL both need the key to decode encrypted
        # HPS scans for the viewer, thumbnails, and the preview pane.
        $cargoArgs += @("--features", "occluview-formats/private-hps-key")
        $shellCargoArgs += @("--features", "occluview-formats/private-hps-key")
    }
    if ($isDiagnosticPackage) {
        $shellCargoArgs += @("--features", "diagnostic-logs")
    }

    $previousEncodedRustFlags = [Environment]::GetEnvironmentVariable("CARGO_ENCODED_RUSTFLAGS")
    $rustFlags = @()
    if ($Configuration -in @("release", "diagnostic")) {
        $separator = [string][char]0x1f
        $rustFlags += "--remap-path-prefix=$repoRoot=occluview"
        $normalizedRepoRoot = $repoRoot.Replace("\", "/")
        if ($normalizedRepoRoot -ne $repoRoot) {
            $rustFlags += "--remap-path-prefix=$normalizedRepoRoot=occluview"
        }
    }
    if ($Target -like "*-pc-windows-msvc") {
        # The MSI runs occluview.exe as a custom action.  Link the MSVC CRT
        # statically so a clean workstation never rolls the installation back
        # before the action can report success.
        $rustFlags += @("-C", "target-feature=+crt-static")
    }
    if ($rustFlags.Count -gt 0) {
        $separator = [string][char]0x1f
        $encodedRustFlags = $rustFlags -join $separator
        if (Test-HasText $previousEncodedRustFlags) {
            $env:CARGO_ENCODED_RUSTFLAGS = "$previousEncodedRustFlags$separator$encodedRustFlags"
        } else {
            $env:CARGO_ENCODED_RUSTFLAGS = $encodedRustFlags
        }
    }
    $previousCmakeToolchain = $null
    $previousBaseCmakeToolchain = $null
    if ($Target -like "*-pc-windows-msvc") {
        $staticCrtToolchain = Join-Path $PSScriptRoot "cmake\\occluview-static-crt.cmake"
        if (-not (Test-Path $staticCrtToolchain)) {
            throw "Missing static-CRT CMake toolchain overlay: $staticCrtToolchain"
        }

        # manifold-csg-sys invokes CMake itself and does not expose a runtime
        # option. Feed it a toolchain overlay before its first project() call
        # so its C++ archives use /MT just like the Rust payload.
        $previousCmakeToolchain = [Environment]::GetEnvironmentVariable("CMAKE_TOOLCHAIN_FILE")
        $previousBaseCmakeToolchain = [Environment]::GetEnvironmentVariable("OCCLUVIEW_BASE_CMAKE_TOOLCHAIN_FILE")
        $env:OCCLUVIEW_BASE_CMAKE_TOOLCHAIN_FILE = $previousCmakeToolchain
        $env:CMAKE_TOOLCHAIN_FILE = $staticCrtToolchain

        # Cargo does not know that CMake's runtime setting changed. Rebuild
        # only Manifold so an older /MD cache cannot be silently reused for an
        # MSI that promises to be self-contained.
        & cargo clean -p manifold-csg-sys
        if ($LASTEXITCODE -ne 0) {
            throw "cargo clean could not invalidate the Manifold CMake build"
        }
    }
    $referenceShellRoot = $null
    $referenceShellAdded = $false
    try {
        & cargo @cargoArgs
        if ($LASTEXITCODE -ne 0) {
            throw "cargo build failed"
        }

        if ($Configuration -eq "release") {
            & git -C $repoRoot cat-file -e "$referenceShellRevision^{commit}"
            if ($LASTEXITCODE -ne 0) {
                throw "The pinned working preview-shell revision $referenceShellRevision is unavailable locally. Fetch full history before building the MSI."
            }
            $referenceParent = if (Test-HasText $env:RUNNER_TEMP) {
                $env:RUNNER_TEMP
            } else {
                [IO.Path]::GetTempPath()
            }
            $referenceShellRoot = Join-Path $referenceParent "occluview-preview-shell-$PID"
            if (Test-Path -LiteralPath $referenceShellRoot) {
                throw "Reference shell worktree path already exists: $referenceShellRoot"
            }
            & git -C $repoRoot worktree add --detach $referenceShellRoot $referenceShellRevision
            if ($LASTEXITCODE -ne 0) {
                throw "git worktree add failed for pinned preview-shell revision $referenceShellRevision"
            }
            $referenceShellAdded = $true
            Set-ReferenceShellPackageVersion -ReferenceRoot $referenceShellRoot -PackageVersion $Version
            $referenceShellBuildDir = Join-Path $referenceShellRoot (Join-Path "target\$Target" "release-unwind")
            $referenceShellCargoArgs = @(
                "build",
                "--locked",
                "-p", "occluview-shell",
                "--target", $Target,
                "--profile", "release-unwind"
            )
            if (Test-HasText $env:OCCLUVIEW_HPS_EMBEDDED_KEY) {
                $referenceShellCargoArgs += @("--features", "occluview-formats/private-hps-key")
            }
            Push-Location $referenceShellRoot
            try {
                & cargo @referenceShellCargoArgs
                if ($LASTEXITCODE -ne 0) {
                    throw "cargo build failed for pinned preview shell"
                }
            } finally {
                Pop-Location
            }
            $referenceShellDll = Join-Path $referenceShellBuildDir "occluview_shell.dll"
            if (-not (Test-Path -LiteralPath $referenceShellDll -PathType Leaf)) {
                throw "Pinned preview shell DLL missing after build: $referenceShellDll"
            }
            $referenceShellFileVersion = (Get-Item -LiteralPath $referenceShellDll).VersionInfo.FileVersion
            if ($referenceShellFileVersion -notin @($Version, "$Version.0")) {
                throw "Reference shell DLL version '$referenceShellFileVersion' does not match MSI version '$Version'."
            }
            New-Item -ItemType Directory -Path $shellBuildDir -Force | Out-Null
            Copy-Item -LiteralPath $referenceShellDll -Destination $shellDllSource -Force
        } else {
            & cargo @shellCargoArgs
            if ($LASTEXITCODE -ne 0) {
                throw "cargo shell build failed"
            }
        }
    } finally {
        if ($referenceShellAdded) {
            & git -C $repoRoot worktree remove --force $referenceShellRoot
            if ($LASTEXITCODE -ne 0) {
                throw "git worktree remove failed for temporary preview shell source: $referenceShellRoot"
            }
        }
        if ($null -eq $previousEncodedRustFlags) {
            Remove-Item Env:CARGO_ENCODED_RUSTFLAGS -ErrorAction SilentlyContinue
        } else {
            $env:CARGO_ENCODED_RUSTFLAGS = $previousEncodedRustFlags
        }
        if ($null -eq $previousCmakeToolchain) {
            Remove-Item Env:CMAKE_TOOLCHAIN_FILE -ErrorAction SilentlyContinue
        } else {
            $env:CMAKE_TOOLCHAIN_FILE = $previousCmakeToolchain
        }
        if ($null -eq $previousBaseCmakeToolchain) {
            Remove-Item Env:OCCLUVIEW_BASE_CMAKE_TOOLCHAIN_FILE -ErrorAction SilentlyContinue
        } else {
            $env:OCCLUVIEW_BASE_CMAKE_TOOLCHAIN_FILE = $previousBaseCmakeToolchain
        }
    }
}

# The wxs harvests both artifacts from one BuildDir: stage the unwind-profile
# DLL next to the exe so the existing -dBuildDir contract stays intact.
# (-SkipBuild reuses a previously staged DLL, so the copy is conditional.)
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
$diagnosticContents = @()
if ($isDiagnosticPackage) {
    $shellPdbSource = Join-Path $shellBuildDir "occluview_shell.pdb"
    $shellPdbDestination = Join-Path $buildDir "occluview_shell.pdb"
    if (-not (Test-Path -LiteralPath $shellPdbSource -PathType Leaf)) {
        throw "Required diagnostic shell PDB missing: $shellPdbSource"
    }
    Copy-Item -LiteralPath $shellPdbSource -Destination $shellPdbDestination -Force

    $diagnosticRequired = @(
        (Join-Path $buildDir "occluview.pdb"),
        $shellPdbDestination
    )
    foreach ($path in $diagnosticRequired) {
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
            throw "Required diagnostic PDB missing: $path"
        }
    }
    $diagnosticContents = @(
        (Get-DiagnosticManifestEntry -Path (Join-Path $buildDir "occluview.exe") -InstallPath "OccluView/occluview.exe" -Kind "binary"),
        (Get-DiagnosticManifestEntry -Path (Join-Path $buildDir "occluview_shell.dll") -InstallPath "OccluView/occluview_shell.dll" -Kind "binary"),
        (Get-DiagnosticManifestEntry -Path (Join-Path $buildDir "occluview.pdb") -InstallPath "OccluView/Diagnostics/occluview.pdb" -Kind "symbol"),
        (Get-DiagnosticManifestEntry -Path $shellPdbDestination -InstallPath "OccluView/Diagnostics/occluview_shell.pdb" -Kind "symbol"),
        (Get-DiagnosticManifestEntry -Path (Join-Path $PSScriptRoot "diagnostics\Enable-PreviewDiagnostics.ps1") -InstallPath "OccluView/Diagnostics/Enable-PreviewDiagnostics.ps1" -Kind "helper"),
        (Get-DiagnosticManifestEntry -Path (Join-Path $PSScriptRoot "diagnostics\Collect-PreviewDiagnostics.ps1") -InstallPath "OccluView/Diagnostics/Collect-PreviewDiagnostics.ps1" -Kind "helper"),
        (Get-DiagnosticManifestEntry -Path (Join-Path $PSScriptRoot "diagnostics\README.txt") -InstallPath "OccluView/Diagnostics/README.txt" -Kind "guide")
    )
}
if ($Target -like "*-pc-windows-msvc") {
    Assert-ManifoldStaticMsvcRuntime -Target $Target -RepoRoot $repoRoot
    Assert-StaticMsvcRuntime -Paths $required
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
    $diagnosticWixDefine,
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

if ($isDiagnosticPackage) {
    Write-DiagnosticManifest `
        -Path $manifestPath `
        -MsiPath $msiPath `
        -Version $Version `
        -Target $Target `
        -Contents $diagnosticContents
}

Write-Host "Built MSI: $msiPath"
