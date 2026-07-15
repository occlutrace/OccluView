[CmdletBinding()]
param(
    [string]$MsiPath = "",
    [string]$UpgradeMsiPath = ""
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$supportedExtensions = @("stl", "ply", "obj", "glb", "dcm", "hps")
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
$prevhostAppId = "{6D2B5079-2F0B-48DD-AB7F-97CEC514D30B}"
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
    Assert-PathExists (Join-Path $startMenuDir "$productName.lnk")

    Assert-Equals (Get-RegistryDefault "HKLM:\Software\Classes\CLSID\$shellClsid") "OccluView Thumbnail Provider" "CLSID friendly name"
    Assert-Equals (Get-RegistryDefault "HKLM:\Software\Classes\CLSID\$shellClsid\InprocServer32") $shellDll "CLSID InprocServer32"
    Assert-Equals (Get-RegistryNamedValue "HKLM:\Software\Classes\CLSID\$shellClsid\InprocServer32" "ThreadingModel") "Apartment" "CLSID threading model"
    Assert-Equals (Get-RegistryNamedValue $approvedShellExtensionsPath $shellClsid) "OccluView Thumbnail Provider" "approved shell extension"
    Assert-Equals (Get-RegistryDefault "HKLM:\Software\Classes\CLSID\$previewClsid") "OccluView Preview Handler" "preview CLSID friendly name"
    Assert-Equals (Get-RegistryNamedValue "HKLM:\Software\Classes\CLSID\$previewClsid" "AppID") $prevhostAppId "preview CLSID AppID"
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
        Assert-Equals (Get-RegistryDefault "HKLM:\Software\Classes\.$ext") $progid ".$ext extension ProgID"
        Assert-Equals (Get-RegistryNamedValue "HKLM:\Software\Classes\.$ext" "ThumbnailCutoff") "1" ".$ext extension thumbnail cutoff"
        Assert-Equals (Get-RegistryNamedValue "HKLM:\Software\Classes\.$ext" "TypeOverlay") "" ".$ext extension thumbnail overlay"
        Assert-Equals (Get-RegistryDefault "HKLM:\Software\Classes\.$ext\DefaultIcon") $defaultIcon ".$ext extension default icon"
        Assert-Equals (Get-RegistryDefault "HKLM:\Software\Classes\.$ext\ShellEx\$thumbnailCategory") $shellClsid ".$ext thumbnail provider"
        Assert-Equals (Get-RegistryDefault "HKLM:\Software\Classes\.$ext\ShellEx\$previewCategory") $previewClsid ".$ext preview handler"
        Assert-Equals (Get-RegistryDefault "$systemFileAssociationsPath\.$ext\ShellEx\$thumbnailCategory") $shellClsid "SystemFileAssociations .$ext thumbnail provider"
        Assert-Equals (Get-RegistryDefault "$systemFileAssociationsPath\.$ext\ShellEx\$previewCategory") $previewClsid "SystemFileAssociations .$ext preview handler"
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

function Assert-UninstalledRegistry {
    Assert-PathAbsent $installDir
    Assert-PathAbsent $startMenuDir
    Assert-PathAbsent "HKLM:\Software\Classes\CLSID\$shellClsid"
    Assert-PathAbsent "HKLM:\Software\Classes\CLSID\$previewClsid"
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
$resolvedUpgradeMsi = if ([string]::IsNullOrWhiteSpace($UpgradeMsiPath)) {
    ""
} else {
    (Resolve-Path $UpgradeMsiPath).Path
}
$installLog = Join-Path $env:TEMP "occluview-msi-install.log"
$upgradeLog = Join-Path $env:TEMP "occluview-msi-upgrade.log"
$uninstallLog = Join-Path $env:TEMP "occluview-msi-uninstall.log"

Write-Host "Installing MSI: $resolvedMsi"
Invoke-MsiExec -Arguments "/i `"$resolvedMsi`" /qn /norestart" -LogPath $installLog
try {
    Assert-InstalledRegistry
    & (Join-Path $PSScriptRoot "test-thumbnail-provider.ps1")
    & (Join-Path $PSScriptRoot "test-preview-handler.ps1") -PreviewClsid $previewClsid
    $productCode = Assert-OneInstalledProduct

    if (-not [string]::IsNullOrWhiteSpace($resolvedUpgradeMsi)) {
        Write-Host "Upgrading MSI: $resolvedUpgradeMsi"
        Invoke-MsiExec -Arguments "/i `"$resolvedUpgradeMsi`" /qn /norestart" -LogPath $upgradeLog
        Assert-InstalledRegistry
        & (Join-Path $PSScriptRoot "test-thumbnail-provider.ps1")
        & (Join-Path $PSScriptRoot "test-preview-handler.ps1") -PreviewClsid $previewClsid
        $productCode = Assert-OneInstalledProduct
    }

    Write-Host "Uninstalling MSI product: $productCode"
    Invoke-MsiExec -Arguments "/x `"$productCode`" /qn /norestart" -LogPath $uninstallLog
    Assert-UninstalledRegistry
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
