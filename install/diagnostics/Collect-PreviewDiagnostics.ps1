[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string]$DestinationDirectory
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

# No elevation is needed. The LocalLow log contains fixed enums only; the
# snapshot below queries only OccluView's fixed registration identities.
$localLow = Join-Path ([Environment]::GetFolderPath([Environment+SpecialFolder]::UserProfile)) "AppData\LocalLow"
$diagnosticDirectory = Join-Path $localLow "OccluView\diagnostics"
$eventLog = Join-Path $diagnosticDirectory "shell-events.jsonl"
$legacyLog = Join-Path $diagnosticDirectory "preview-failures.jsonl"
$availableLogs = @($eventLog, $legacyLog) | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf }
if ($availableLogs.Count -eq 0) {
    throw "No OccluView shell diagnostic log was found for the current user."
}

$thumbnailClsid = "{9F3A1B2C-4D5E-4F60-8A7B-9C0D1E2F3045}"
$previewClsid = "{9F3A1B2C-4D5E-4F60-8A7B-9C0D1E2F3046}"
$prevhostAppId = "{FD67C578-DBCC-4E10-8E47-63A8E48F7654}"
$thumbnailCategory = "{E357FCCD-A995-4576-B01F-234630154E96}"
$previewCategory = "{8895B1C6-B41F-4C1C-A562-0D564250836F}"
$extensions = @(".stl", ".ply", ".obj", ".glb", ".hps", ".dcm")

New-Item -ItemType Directory -Path $DestinationDirectory -Force | Out-Null
$stamp = Get-Date -Format "yyyyMMdd-HHmmss"
$staging = Join-Path ([IO.Path]::GetTempPath()) "occluview-shell-diagnostics-$stamp"
$archive = Join-Path $DestinationDirectory "OccluView-shell-diagnostics-$stamp.zip"

function Add-RegistryQuery {
    param(
        [Parameter(Mandatory = $true)][System.IO.StreamWriter]$Writer,
        [Parameter(Mandatory = $true)][string]$Label,
        [Parameter(Mandatory = $true)][string]$Key,
        [string]$View = ""
    )

    $Writer.WriteLine("=== $Label : $Key $View ===")
    $arguments = @("query", $Key, "/s")
    if (-not [string]::IsNullOrWhiteSpace($View)) {
        $arguments += $View
    }
    $result = & reg.exe @arguments 2>&1
    foreach ($line in $result) {
        $Writer.WriteLine($line)
    }
    $Writer.WriteLine()
}

try {
    New-Item -ItemType Directory -Path $staging -Force | Out-Null
    foreach ($log in $availableLogs) {
        Copy-Item -LiteralPath $log -Destination (Join-Path $staging (Split-Path -Leaf $log)) -Force
    }

    $snapshot = Join-Path $staging "shell-registration.txt"
    $writer = [System.IO.StreamWriter]::new($snapshot, $false, [System.Text.UTF8Encoding]::new($false))
    try {
        $roots = @(
            @{ Label = "HKCU"; Prefix = "HKCU\Software\Classes"; View = "" },
            @{ Label = "HKLM64"; Prefix = "HKLM\Software\Classes"; View = "/reg:64" },
            @{ Label = "HKLM32"; Prefix = "HKLM\Software\Classes"; View = "/reg:32" },
            @{ Label = "HKCR"; Prefix = "HKCR"; View = "" }
        )
        foreach ($root in $roots) {
            foreach ($fixedKey in @(
                "CLSID\$thumbnailClsid",
                "CLSID\$previewClsid",
                "AppID\$prevhostAppId"
            )) {
                Add-RegistryQuery -Writer $writer -Label $root.Label -Key "$($root.Prefix)\$fixedKey" -View $root.View
            }
            foreach ($extension in $extensions) {
                Add-RegistryQuery -Writer $writer -Label $root.Label -Key "$($root.Prefix)\$extension\ShellEx\$thumbnailCategory" -View $root.View
                Add-RegistryQuery -Writer $writer -Label $root.Label -Key "$($root.Prefix)\$extension\ShellEx\$previewCategory" -View $root.View
            }
        }
    } finally {
        $writer.Dispose()
    }
    Compress-Archive -Path (Join-Path $staging "*") -DestinationPath $archive -Force
} finally {
    Remove-Item -LiteralPath $staging -Recurse -Force -ErrorAction SilentlyContinue
}

Write-Host "Collected OccluView shell diagnostics: $archive"
