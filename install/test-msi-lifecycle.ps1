[CmdletBinding()]
param(
    [string]$MsiPath = "",
    [string]$LegacyUpgradeMsiPath = "",
    [string]$UpgradeMsiPath = "",
    [string]$DowngradeMsiPath = "",
    [switch]$Diagnostic
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$supportedExtensions = @("stl", "ply", "obj", "glb", "dcm", "hps")
# Extensions the installer may claim machine-wide: default ProgID, DefaultIcon
# and the ShellEx handlers under both the bare key and SystemFileAssociations.
$ownedExtensions = @("stl", "ply", "obj", "glb", "hps")
# Read, offered in "Open with", never claimed. .dcm belongs to medical DICOM as
# much as it does to 3Shape's HPS container, and this reader rejects DICOM, so
# owning it would put a failing thumbnail on every CBCT file on the workstation.
$offeredOnlyExtensions = @("dcm")
$deferredExtensions = @("gltf", "3mf")
$formatProgIds = @{
    stl = "MeshFile.STL"
    ply = "MeshFile.PLY"
    obj = "MeshFile.OBJ"
    glb = "MeshFile.GLB"
    dcm = "MeshFile.HPS"
    hps = "MeshFile.HPS"
}
$formatFriendlyNames = @{
    "MeshFile.STL" = "STL File"
    "MeshFile.PLY" = "PLY File"
    "MeshFile.OBJ" = "OBJ File"
    "MeshFile.GLB" = "GLB File"
    "MeshFile.HPS" = "HPS File"
}
$legacyFormatProgIds = @{
    stl = "OccluView.Mesh.STL"
    ply = "OccluView.Mesh.PLY"
    obj = "OccluView.Mesh.OBJ"
    glb = "OccluView.Mesh.GLB"
    dcm = "OccluView.Mesh.HPS"
    hps = "OccluView.Mesh.HPS"
}
$thumbnailCategory = "{E357FCCD-A995-4576-B01F-234630154E96}"
$previewCategory = "{8895B1C6-B41F-4C1C-A562-0D564250836F}"
$shellClsid = "{9F3A1B2C-4D5E-4F60-8A7B-9C0D1E2F3045}"
$previewClsid = "{9F3A1B2C-4D5E-4F60-8A7B-9C0D1E2F3046}"
$windowsDefaultPrevhostAppId = "{6D2B5079-2F0B-48DD-AB7F-97CEC514D30B}"
$prevhostAppId = "{FD67C578-DBCC-4E10-8E47-63A8E48F7654}"
$previewAppIdPath = "HKLM:\Software\Classes\AppID\$prevhostAppId"
$productName = "OccluView 3D Viewer"
$capabilitiesPath = "HKLM:\Software\OccluTrace\OccluView\Capabilities"
$fileAssociationsPath = "$capabilitiesPath\FileAssociations"
$applicationsPath = "HKLM:\Software\Classes\Applications\occluview.exe"
$systemFileAssociationsPath = "HKLM:\Software\Classes\SystemFileAssociations"
$approvedShellExtensionsPath = "HKLM:\Software\Microsoft\Windows\CurrentVersion\Shell Extensions\Approved"
$previewHandlersPath = "HKLM:\Software\Microsoft\Windows\CurrentVersion\PreviewHandlers"
$installDir = Join-Path ${env:ProgramFiles} "OccluView"
$appExe = Join-Path $installDir "occluview.exe"
$shellDll = Join-Path $installDir "occluview_shell.dll"
$diagnosticDir = Join-Path $installDir "Diagnostics"
$diagnosticRegistryPath = "HKCU:\Software\OccluTrace\OccluView\Diagnostics"
$formatIconFiles = @{
    stl = Join-Path $installDir "occluview-3d.ico"
    ply = Join-Path $installDir "occluview-3d.ico"
    obj = Join-Path $installDir "occluview-3d.ico"
    glb = Join-Path $installDir "occluview-3d.ico"
    dcm = Join-Path $installDir "occluview-3d.ico"
    hps = Join-Path $installDir "occluview-3d.ico"
}
$formatDefaultIcons = @{
    stl = Join-Path $installDir "occluview-3d.ico"
    ply = Join-Path $installDir "occluview-3d.ico"
    obj = Join-Path $installDir "occluview-3d.ico"
    glb = Join-Path $installDir "occluview-3d.ico"
    dcm = Join-Path $installDir "occluview-3d.ico"
    hps = Join-Path $installDir "occluview-3d.ico"
}
$startMenuDir = Join-Path ${env:ProgramData} "Microsoft\Windows\Start Menu\Programs\OccluView"

function Resolve-MsiPath {
    param([string]$Path)

    if (-not [string]::IsNullOrWhiteSpace($Path)) {
        return (Resolve-Path $Path).Path
    }

    $repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
    $candidate = Get-ChildItem -Path (Join-Path $repoRoot "dist") -Filter "*.msi" -File |
        Sort-Object LastWriteTime -Descending |
        Select-Object -First 1
    if ($null -eq $candidate) {
        throw "No MSI found under dist/. Pass -MsiPath explicitly."
    }
    return $candidate.FullName
}

function Invoke-MsiExec {
    param(
        [Parameter(Mandatory = $true)][string]$Arguments,
        [Parameter(Mandatory = $true)][string]$LogPath
    )

    $process = Start-Process -FilePath "msiexec.exe" -ArgumentList "$Arguments /l*v `"$LogPath`"" -Wait -PassThru
    if ($process.ExitCode -ne 0 -and $process.ExitCode -ne 3010) {
        if (Test-Path $LogPath) {
            Get-Content $LogPath -Tail 120 | Write-Host
        }
        throw "msiexec failed with exit code $($process.ExitCode). Log: $LogPath"
    }
}

function Invoke-MsiExecExpectFailure {
    param(
        [Parameter(Mandatory = $true)][string]$Arguments,
        [Parameter(Mandatory = $true)][string]$LogPath
    )

    $process = Start-Process -FilePath "msiexec.exe" -ArgumentList "$Arguments /l*v `"$LogPath`"" -Wait -PassThru
    if ($process.ExitCode -ne 1603) {
        if (Test-Path $LogPath) {
            Get-Content $LogPath -Tail 120 | Write-Host
        }
        throw "Expected Windows Installer downgrade block 1603, got $($process.ExitCode). Log: $LogPath"
    }
    Write-Host "Blocked downgrade exited as expected with 1603."
}

function Start-ActivePreviewHost {
    $readyMarker = "PREVIEW_HOLD_READY"
    $workDir = Join-Path $env:TEMP ("occluview-preview-upgrade-" + [guid]::NewGuid().ToString("N"))
    $stdoutPath = Join-Path $workDir "preview.stdout.log"
    $stderrPath = Join-Path $workDir "preview.stderr.log"
    $process = $null
    New-Item -ItemType Directory -Path $workDir -Force | Out-Null

    try {
        $currentHost = Get-Process -Id $PID -ErrorAction Stop
        $previewSmokePath = Join-Path $PSScriptRoot "test-preview-handler.ps1"
        $startArgs = @{
            FilePath = $currentHost.Path
            ArgumentList = @("-NoLogo", "-NoProfile", "-NonInteractive", "-File", ('"{0}"' -f $previewSmokePath), "-HoldOpenSeconds", "90")
            RedirectStandardOutput = $stdoutPath
            RedirectStandardError = $stderrPath
            PassThru = $true
        }
        $process = Start-Process @startArgs

        $deadline = [Diagnostics.Stopwatch]::StartNew()
        while ($deadline.Elapsed.TotalSeconds -lt 45) {
            $process.Refresh()
            if ($process.HasExited) {
                $stdout = if (Test-Path $stdoutPath) { Get-Content -Raw $stdoutPath } else { "" }
                $stderr = if (Test-Path $stderrPath) { Get-Content -Raw $stderrPath } else { "" }
                throw "Preview-holder exited before activation (exit $($process.ExitCode)). stdout: $stdout stderr: $stderr"
            }
            if ((Test-Path $stdoutPath) -and (Select-String -Path $stdoutPath -SimpleMatch -Quiet $readyMarker)) {
                return [pscustomobject]@{
                    Process = $process
                    WorkDir = $workDir
                    StdoutPath = $stdoutPath
                    StderrPath = $stderrPath
                }
            }
            Start-Sleep -Milliseconds 250
        }
        throw "Preview-holder did not report $readyMarker within 45 seconds."
    } catch {
        if ($null -ne $process -and -not $process.HasExited) {
            Stop-Process -Id $process.Id -Force
        }
        Remove-Item -LiteralPath $workDir -Recurse -Force -ErrorAction SilentlyContinue
        throw
    }
}

function Assert-ActivePreviewHost {
    param([Parameter(Mandatory = $true)]$Holder)

    $Holder.Process.Refresh()
    if ($Holder.Process.HasExited) {
        $stdout = if (Test-Path $Holder.StdoutPath) { Get-Content -Raw $Holder.StdoutPath } else { "" }
        $stderr = if (Test-Path $Holder.StderrPath) { Get-Content -Raw $Holder.StderrPath } else { "" }
        throw "Preview-holder exited during the MSI upgrade (exit $($Holder.Process.ExitCode)). stdout: $stdout stderr: $stderr"
    }
}

function Stop-ActivePreviewHost {
    param([Parameter(Mandatory = $true)]$Holder)

    if ($null -ne $Holder.Process) {
        $Holder.Process.Refresh()
        if (-not $Holder.Process.HasExited) {
            Stop-Process -Id $Holder.Process.Id -Force
            $Holder.Process.WaitForExit()
        }
    }
    Remove-Item -LiteralPath $Holder.WorkDir -Recurse -Force -ErrorAction SilentlyContinue
}

function Get-RegistryDefault {
    param([Parameter(Mandatory = $true)][string]$Path)

    $item = Get-Item -Path $Path -ErrorAction Stop
    return $item.GetValue("")
}

function Get-RegistryNamedValue {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Name
    )

    $item = Get-Item -Path $Path -ErrorAction Stop
    return $item.GetValue($Name, $null)
}

function Assert-PathExists {
    param([Parameter(Mandatory = $true)][string]$Path)

    if (-not (Test-Path $Path)) {
        throw "Expected path to exist: $Path"
    }
}

function Assert-PathAbsent {
    param([Parameter(Mandatory = $true)][string]$Path)

    if (Test-Path $Path) {
        throw "Expected path to be absent: $Path"
    }
}

function Assert-Equals {
    param(
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$Actual,
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$Expected,
        [Parameter(Mandatory = $true)][string]$Label
    )

    if ($Actual -ne $Expected) {
        throw "$Label mismatch. Expected '$Expected', got '$Actual'."
    }
}

function Assert-RegistryDefaultNotEquals {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$Forbidden,
        [Parameter(Mandatory = $true)][string]$Label
    )

    if (-not (Test-Path $Path)) {
        return
    }

    $actual = Get-RegistryDefault $Path
    if ($actual -eq $Forbidden) {
        throw "$Label must not be '$Forbidden'."
    }
}

function Assert-RegistryNamedValueAbsent {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string]$Label
    )

    if (-not (Test-Path $Path)) {
        return
    }

    $value = Get-RegistryNamedValue $Path $Name
    if ($null -ne $value) {
        throw "$Label must be absent, got '$value'."
    }
}

function Get-DiagnosticSwitchState {
    $keyExists = Test-Path $diagnosticRegistryPath
    $value = if ($keyExists) {
        Get-RegistryNamedValue $diagnosticRegistryPath "PreviewFailureLogEnabled"
    } else {
        $null
    }
    return [pscustomobject]@{
        KeyExists = $keyExists
        Value = $value
    }
}

function Assert-DiagnosticSwitchUnchanged {
    param([Parameter(Mandatory = $true)]$Expected)

    $actual = Get-DiagnosticSwitchState
    if ($actual.KeyExists -ne $Expected.KeyExists -or $actual.Value -ne $Expected.Value) {
        throw "Diagnostic MSI changed the current user's PreviewFailureLogEnabled switch."
    }
}

function Assert-DiagnosticPayload {
    foreach ($path in @(
        (Join-Path $diagnosticDir "occluview.pdb"),
        (Join-Path $diagnosticDir "occluview_shell.pdb"),
        (Join-Path $diagnosticDir "Enable-PreviewDiagnostics.ps1"),
        (Join-Path $diagnosticDir "Collect-PreviewDiagnostics.ps1"),
        (Join-Path $diagnosticDir "README.txt")
    )) {
        Assert-PathExists $path
    }
}

function Find-InstalledProductCode {
    $codes = @(Find-InstalledProductCodes)
    if ($codes.Count -eq 0) {
        return $null
    }
    return $codes[0]
}

function Find-InstalledProductCodes {
    $roots = @(
        "HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall",
        "HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall"
    )

    $codes = @()
    foreach ($root in $roots) {
        if (-not (Test-Path $root)) {
            continue
        }
        foreach ($child in Get-ChildItem $root) {
            $displayName = $child.GetValue("DisplayName", $null)
            if (($displayName -eq $productName) -and ($child.PSChildName -match '^\{[0-9A-Fa-f-]{36}\}$')) {
                $codes += $child.PSChildName
            }
        }
    }
    return $codes
}

function Assert-OneInstalledProduct {
    $codes = @(Find-InstalledProductCodes)
    if ($codes.Count -ne 1) {
        throw "Expected exactly one installed OccluView product, found $($codes.Count): $($codes -join ', ')"
    }
    return $codes[0]
}

function Assert-NoInstalledProducts {
    $codes = @(Find-InstalledProductCodes)
    if ($codes.Count -ne 0) {
        throw "Expected OccluView MSI product registration to be gone, found $($codes.Count): $($codes -join ', ')"
    }
}

function Assert-InstalledRegistry {
    Assert-PathExists $appExe
    Assert-PathExists $shellDll
    Assert-PathAbsent (Join-Path $installDir "occluview-cli.exe")
    Assert-PathExists (Join-Path $installDir "LICENSE")
    Assert-PathExists (Join-Path $installDir "NOTICE")
    Assert-PathExists (Join-Path $installDir "THIRD-PARTY-NOTICES.md")
    Assert-PathExists (Join-Path $startMenuDir "$productName.lnk")

    Assert-Equals (Get-RegistryDefault "HKLM:\Software\Classes\CLSID\$shellClsid") "OccluView Thumbnail Provider" "CLSID friendly name"
    Assert-Equals (Get-RegistryDefault "HKLM:\Software\Classes\CLSID\$shellClsid\InprocServer32") $shellDll "CLSID InprocServer32"
    Assert-Equals (Get-RegistryNamedValue "HKLM:\Software\Classes\CLSID\$shellClsid\InprocServer32" "ThreadingModel") "Apartment" "CLSID threading model"
    Assert-Equals (Get-RegistryNamedValue $approvedShellExtensionsPath $shellClsid) "OccluView Thumbnail Provider" "approved shell extension"
    Assert-Equals (Get-RegistryDefault "HKLM:\Software\Classes\CLSID\$previewClsid") "OccluView Preview Handler" "preview CLSID friendly name"
    Assert-Equals (Get-RegistryNamedValue "HKLM:\Software\Classes\CLSID\$previewClsid" "AppID") $prevhostAppId "preview CLSID AppID"
    Assert-PathExists $previewAppIdPath
    # Windows Installer may canonicalize this documented surrogate executable
    # to its System32 path. Accept only the documented basename or that exact
    # system executable: any other path would select a different surrogate.
    $previewSurrogate = Get-RegistryNamedValue $previewAppIdPath "DllSurrogate"
    $allowedPreviewSurrogates = @(
        "Prevhost.exe",
        (Join-Path $env:WINDIR "System32\Prevhost.exe")
    )
    if ($previewSurrogate -notin $allowedPreviewSurrogates) {
        throw "preview AppID DllSurrogate must be Prevhost.exe or its System32 path, got '$previewSurrogate'."
    }
    Assert-RegistryNamedValueAbsent "HKLM:\Software\Classes\CLSID\$previewClsid" "DisableLowILProcessIsolation" "preview low-integrity isolation override"
    Assert-Equals (Get-RegistryDefault "HKLM:\Software\Classes\CLSID\$previewClsid\InprocServer32") $shellDll "preview CLSID InprocServer32"
    Assert-Equals (Get-RegistryNamedValue "HKLM:\Software\Classes\CLSID\$previewClsid\InprocServer32" "ThreadingModel") "Apartment" "preview CLSID threading model"
    Assert-Equals (Get-RegistryNamedValue $approvedShellExtensionsPath $previewClsid) "OccluView Preview Handler" "approved preview shell extension"
    Assert-Equals (Get-RegistryNamedValue $previewHandlersPath $previewClsid) "OccluView Preview Handler" "PreviewHandlers entry"
    Assert-PathAbsent "HKLM:\Software\Classes\OccluView.Mesh"
    foreach ($legacyProgid in $legacyFormatProgIds.Values) {
        Assert-PathAbsent "HKLM:\Software\Classes\$legacyProgid"
    }
    Assert-Equals (Get-RegistryDefault $applicationsPath) $productName "Applications friendly name"
    Assert-Equals (Get-RegistryNamedValue $applicationsPath "FriendlyAppName") $productName "Applications FriendlyAppName"
    Assert-Equals (Get-RegistryDefault "$applicationsPath\shell\open\command") "`"$appExe`" `"%1`"" "Applications open command"
    Assert-Equals (Get-RegistryNamedValue "HKLM:\Software\RegisteredApplications" "OccluView") "Software\OccluTrace\OccluView\Capabilities" "RegisteredApplications entry"
    Assert-Equals (Get-RegistryNamedValue $capabilitiesPath "ApplicationName") $productName "Capabilities ApplicationName"
    Assert-Equals (Get-RegistryNamedValue $capabilitiesPath "ApplicationIcon") "$appExe,0" "Capabilities ApplicationIcon"

    foreach ($ext in $supportedExtensions) {
        $progid = $formatProgIds[$ext]
        $formatIcon = $formatIconFiles[$ext]
        $defaultIcon = $formatDefaultIcons[$ext]
        $friendlyName = $formatFriendlyNames[$progid]
        Assert-PathExists $formatIcon
        Assert-Equals (Get-RegistryDefault "HKLM:\Software\Classes\$progid") $friendlyName "$progid friendly name"
        Assert-Equals (Get-RegistryNamedValue "HKLM:\Software\Classes\$progid" "ThumbnailCutoff") "1" "$progid thumbnail cutoff"
        Assert-Equals (Get-RegistryNamedValue "HKLM:\Software\Classes\$progid" "TypeOverlay") "" "$progid thumbnail overlay"
        Assert-Equals (Get-RegistryDefault "HKLM:\Software\Classes\$progid\DefaultIcon") $defaultIcon "$progid default icon"
        Assert-Equals (Get-RegistryDefault "HKLM:\Software\Classes\$progid\ShellEx\$thumbnailCategory") $shellClsid "$progid thumbnail provider"
        Assert-Equals (Get-RegistryDefault "HKLM:\Software\Classes\$progid\ShellEx\$previewCategory") $previewClsid "$progid preview handler"
        Assert-Equals (Get-RegistryDefault "HKLM:\Software\Classes\$progid\shell\open\command") "`"$appExe`" `"%1`"" "$progid open command"
        if ($ownedExtensions -contains $ext) {
            Assert-Equals (Get-RegistryDefault "HKLM:\Software\Classes\.$ext") $progid ".$ext extension ProgID"
            Assert-Equals (Get-RegistryNamedValue "HKLM:\Software\Classes\.$ext" "ThumbnailCutoff") "1" ".$ext extension thumbnail cutoff"
            Assert-Equals (Get-RegistryNamedValue "HKLM:\Software\Classes\.$ext" "TypeOverlay") "" ".$ext extension thumbnail overlay"
            Assert-Equals (Get-RegistryDefault "HKLM:\Software\Classes\.$ext\DefaultIcon") $defaultIcon ".$ext extension default icon"
            Assert-Equals (Get-RegistryDefault "HKLM:\Software\Classes\.$ext\ShellEx\$thumbnailCategory") $shellClsid ".$ext thumbnail provider"
            Assert-Equals (Get-RegistryDefault "HKLM:\Software\Classes\.$ext\ShellEx\$previewCategory") $previewClsid ".$ext preview handler"
            Assert-Equals (Get-RegistryDefault "$systemFileAssociationsPath\.$ext\ShellEx\$thumbnailCategory") $shellClsid "SystemFileAssociations .$ext thumbnail provider"
            Assert-Equals (Get-RegistryDefault "$systemFileAssociationsPath\.$ext\ShellEx\$previewCategory") $previewClsid "SystemFileAssociations .$ext preview handler"
        } elseif ($offeredOnlyExtensions -contains $ext) {
            # A fresh install must leave every machine-wide .dcm surface alone.
            # These are the exact values a DICOM viewer owns, so asserting we
            # are not in them is asserting we did not take the extension.
            Assert-RegistryDefaultNotEquals "HKLM:\Software\Classes\.$ext" $progid ".$ext extension ProgID"
            Assert-RegistryDefaultNotEquals "HKLM:\Software\Classes\.$ext\DefaultIcon" $defaultIcon ".$ext extension default icon"
            Assert-RegistryDefaultNotEquals "HKLM:\Software\Classes\.$ext\ShellEx\$thumbnailCategory" $shellClsid ".$ext thumbnail provider"
            Assert-RegistryDefaultNotEquals "HKLM:\Software\Classes\.$ext\ShellEx\$previewCategory" $previewClsid ".$ext preview handler"
            Assert-RegistryDefaultNotEquals "$systemFileAssociationsPath\.$ext\ShellEx\$thumbnailCategory" $shellClsid "SystemFileAssociations .$ext thumbnail provider"
            Assert-RegistryDefaultNotEquals "$systemFileAssociationsPath\.$ext\ShellEx\$previewCategory" $previewClsid "SystemFileAssociations .$ext preview handler"
        } else {
            throw "Extension .$ext is in neither the owned nor the offered-only list."
        }
        # Offered on every supported extension: this is how a user reaches
        # OccluView deliberately, and it takes nothing away from anyone.
        Assert-Equals (Get-RegistryNamedValue "HKLM:\Software\Classes\.$ext\OpenWithProgids" $progid) "" ".$ext OpenWithProgids"
        Assert-Equals (Get-RegistryDefault "HKLM:\Software\Classes\.$ext\OpenWithList\occluview.exe") "" ".$ext OpenWithList"
        Assert-Equals (Get-RegistryNamedValue "$applicationsPath\SupportedTypes" ".$ext") "" ".$ext Applications SupportedTypes"
        Assert-Equals (Get-RegistryNamedValue $fileAssociationsPath ".$ext") $progid ".$ext Capabilities FileAssociations"
    }

    foreach ($ext in $deferredExtensions) {
        Assert-RegistryDefaultNotEquals "HKLM:\Software\Classes\.$ext\ShellEx\$thumbnailCategory" $shellClsid ".$ext thumbnail provider"
        Assert-RegistryDefaultNotEquals "HKLM:\Software\Classes\.$ext\ShellEx\$previewCategory" $previewClsid ".$ext preview handler"
        $openWithPath = "HKLM:\Software\Classes\.$ext\OpenWithProgids"
        if (Test-Path $openWithPath) {
            foreach ($progid in $formatProgIds.Values) {
                $value = Get-RegistryNamedValue $openWithPath $progid
                if ($null -ne $value) {
                    throw "Deferred extension .$ext must not register $progid in OpenWithProgids."
                }
            }
        }
    }
}

function Assert-LegacyPreviewHostRegistration {
    # The historical 1.1.1 baseline referenced Windows' generic preview-host
    # AppID but did not own that AppID key. This is intentionally narrower
    # than Assert-InstalledRegistry: the next install must migrate this value
    # without treating the Windows key as OccluView-owned state.
    Assert-Equals (Get-RegistryNamedValue "HKLM:\Software\Classes\CLSID\$previewClsid" "AppID") $windowsDefaultPrevhostAppId "legacy preview CLSID AppID"
    Assert-PathAbsent $previewAppIdPath
}

function Assert-UninstalledRegistry {
    Assert-PathAbsent $installDir
    Assert-PathAbsent $startMenuDir
    Assert-PathAbsent "HKLM:\Software\Classes\CLSID\$shellClsid"
    Assert-PathAbsent "HKLM:\Software\Classes\CLSID\$previewClsid"
    Assert-PathAbsent $previewAppIdPath
    Assert-PathAbsent "HKLM:\Software\Classes\OccluView.Mesh"
    foreach ($legacyProgid in $legacyFormatProgIds.Values) {
        Assert-PathAbsent "HKLM:\Software\Classes\$legacyProgid"
    }
    foreach ($progid in $formatProgIds.Values) {
        Assert-PathAbsent "HKLM:\Software\Classes\$progid"
    }
    foreach ($formatIcon in $formatIconFiles.Values) {
        Assert-PathAbsent $formatIcon
    }
    Assert-PathAbsent $applicationsPath
    Assert-RegistryNamedValueAbsent $approvedShellExtensionsPath $shellClsid "approved shell extension"
    Assert-RegistryNamedValueAbsent $approvedShellExtensionsPath $previewClsid "approved preview shell extension"
    Assert-RegistryNamedValueAbsent $previewHandlersPath $previewClsid "PreviewHandlers entry"
    Assert-RegistryNamedValueAbsent "HKLM:\Software\RegisteredApplications" "OccluView" "RegisteredApplications entry"
    Assert-PathAbsent "HKLM:\Software\OccluTrace\OccluView"

    foreach ($ext in $supportedExtensions) {
        $progid = $formatProgIds[$ext]
        Assert-RegistryDefaultNotEquals "HKLM:\Software\Classes\.$ext\ShellEx\$thumbnailCategory" $shellClsid ".$ext thumbnail provider"
        Assert-RegistryDefaultNotEquals "HKLM:\Software\Classes\.$ext\ShellEx\$previewCategory" $previewClsid ".$ext preview handler"
        Assert-RegistryDefaultNotEquals "$systemFileAssociationsPath\.$ext\ShellEx\$thumbnailCategory" $shellClsid "SystemFileAssociations .$ext thumbnail provider"
        Assert-RegistryDefaultNotEquals "$systemFileAssociationsPath\.$ext\ShellEx\$previewCategory" $previewClsid "SystemFileAssociations .$ext preview handler"
        Assert-RegistryDefaultNotEquals "HKLM:\Software\Classes\.$ext" $progid ".$ext extension ProgID"
        Assert-RegistryDefaultNotEquals "HKLM:\Software\Classes\.$ext\DefaultIcon" $formatDefaultIcons[$ext] ".$ext extension default icon"
        $openWithPath = "HKLM:\Software\Classes\.$ext\OpenWithProgids"
        if (Test-Path $openWithPath) {
            $value = Get-RegistryNamedValue $openWithPath $progid
            if ($null -ne $value) {
                throw "Uninstall left $progid under .$ext OpenWithProgids."
            }
        }
        Assert-PathAbsent "HKLM:\Software\Classes\.$ext\OpenWithList\occluview.exe"
    }
}

$resolvedMsi = Resolve-MsiPath $MsiPath
$resolvedLegacyUpgradeMsi = if ([string]::IsNullOrWhiteSpace($LegacyUpgradeMsiPath)) {
    ""
} else {
    (Resolve-Path $LegacyUpgradeMsiPath).Path
}
$resolvedUpgradeMsi = if ([string]::IsNullOrWhiteSpace($UpgradeMsiPath)) {
    ""
} else {
    (Resolve-Path $UpgradeMsiPath).Path
}
$resolvedDowngradeMsi = if ([string]::IsNullOrWhiteSpace($DowngradeMsiPath)) {
    ""
} else {
    (Resolve-Path $DowngradeMsiPath).Path
}
$installLog = Join-Path $env:TEMP "occluview-msi-install.log"
$legacyInstallLog = Join-Path $env:TEMP "occluview-msi-legacy-install.log"
$upgradeLog = Join-Path $env:TEMP "occluview-msi-upgrade.log"
$downgradeLog = Join-Path $env:TEMP "occluview-msi-downgrade.log"
$uninstallLog = Join-Path $env:TEMP "occluview-msi-uninstall.log"
$diagnosticSwitchBefore = if ($Diagnostic) { Get-DiagnosticSwitchState } else { $null }
$legacyProductCode = $null
try {
    if (-not [string]::IsNullOrWhiteSpace($resolvedLegacyUpgradeMsi)) {
        Write-Host "Installing pinned legacy MSI: $resolvedLegacyUpgradeMsi"
        Invoke-MsiExec -Arguments "/i `"$resolvedLegacyUpgradeMsi`" /qn /norestart" -LogPath $legacyInstallLog
        Assert-LegacyPreviewHostRegistration
        $legacyProductCode = Assert-OneInstalledProduct
        Write-Host "Migrating pinned legacy MSI: $resolvedLegacyUpgradeMsi -> $resolvedMsi"
    } else {
        Write-Host "Installing MSI: $resolvedMsi"
    }
    Invoke-MsiExec -Arguments "/i `"$resolvedMsi`" /qn /norestart" -LogPath $installLog
    Assert-InstalledRegistry
    if ($Diagnostic) {
        Assert-DiagnosticPayload
        Assert-DiagnosticSwitchUnchanged $diagnosticSwitchBefore
    }
    & (Join-Path $PSScriptRoot "test-thumbnail-provider.ps1")
    & (Join-Path $PSScriptRoot "test-preview-handler.ps1") -PreviewClsid $previewClsid
    $productCode = Assert-OneInstalledProduct
    if ($null -ne $legacyProductCode -and $productCode -eq $legacyProductCode) {
        throw "Legacy MSI upgrade kept product code $productCode instead of completing a major upgrade."
    }

    if (-not [string]::IsNullOrWhiteSpace($resolvedUpgradeMsi)) {
        Write-Host "Upgrading MSI: $resolvedUpgradeMsi"
        $previousProductCode = $productCode
        $previewHolder = Start-ActivePreviewHost
        try {
            Assert-ActivePreviewHost $previewHolder
            Invoke-MsiExec -Arguments "/i `"$resolvedUpgradeMsi`" /qn /norestart" -LogPath $upgradeLog
            Assert-ActivePreviewHost $previewHolder
        } finally {
            Stop-ActivePreviewHost $previewHolder
        }
        Assert-InstalledRegistry
        & (Join-Path $PSScriptRoot "test-thumbnail-provider.ps1")
        & (Join-Path $PSScriptRoot "test-preview-handler.ps1") -PreviewClsid $previewClsid
        $productCode = Assert-OneInstalledProduct
        if ($productCode -eq $previousProductCode) {
            throw "Major upgrade kept product code $productCode instead of replacing the prior product."
        }
    }

    if (-not [string]::IsNullOrWhiteSpace($resolvedDowngradeMsi)) {
        Write-Host "Attempting blocked downgrade MSI: $resolvedDowngradeMsi"
        $productCodeBeforeDowngrade = $productCode
        Invoke-MsiExecExpectFailure -Arguments "/i `"$resolvedDowngradeMsi`" /qn /norestart" -LogPath $downgradeLog
        Assert-InstalledRegistry
        $productCode = Assert-OneInstalledProduct
        if ($productCode -ne $productCodeBeforeDowngrade) {
            throw "Blocked downgrade replaced product code $productCodeBeforeDowngrade with $productCode."
        }
    }

    Write-Host "Uninstalling MSI product: $productCode"
    Invoke-MsiExec -Arguments "/x `"$productCode`" /qn /norestart" -LogPath $uninstallLog
    if (Test-Path $previewAppIdPath) {
        $surrogate = Get-RegistryNamedValue $previewAppIdPath "DllSurrogate"
        $machineQuery = (& reg.exe query "HKLM\Software\Classes\AppID\$prevhostAppId" /s 2>&1 | Out-String).Trim()
        $wowQuery = (& reg.exe query "HKLM\Software\Classes\WOW6432Node\AppID\$prevhostAppId" /s 2>&1 | Out-String).Trim()
        $msiEvidence = Select-String -Path $uninstallLog -Pattern "cmpPreviewHostRegistration|RemoveRegistryValues|WriteRegistryValues|RefreshShellAssociationsUninstall|DllSurrogate|$prevhostAppId" | ForEach-Object { $_.Line }
        Write-Host "Uninstall left preview AppID: DllSurrogate='$surrogate'; machine=$machineQuery; wow6432=$wowQuery"
        Write-Host "Uninstall AppID action evidence: $($msiEvidence -join [Environment]::NewLine)"
    }
    Assert-UninstalledRegistry
    if ($Diagnostic) {
        Assert-DiagnosticSwitchUnchanged $diagnosticSwitchBefore
    }
    Assert-NoInstalledProducts
} catch {
    $productCode = Find-InstalledProductCode
    if ($null -ne $productCode) {
        Write-Warning "Smoke failed; attempting cleanup uninstall for $productCode"
        Start-Process -FilePath "msiexec.exe" -ArgumentList "/x `"$productCode`" /qn /norestart" -Wait | Out-Null
    }
    throw
}

Write-Host "MSI lifecycle smoke passed."
