[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string]$DestinationDirectory
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

# The shell writes only its fixed-field JSONL under LocalLow. Collection copies
# that one file to a caller-selected directory and never asks for elevation.
$localLow = Join-Path ([Environment]::GetFolderPath([Environment+SpecialFolder]::UserProfile)) "AppData\LocalLow"
$source = Join-Path $localLow "OccluView\diagnostics\preview-failures.jsonl"
if (-not (Test-Path -LiteralPath $source -PathType Leaf)) {
    throw "No OccluView preview diagnostic log was found for the current user."
}

New-Item -ItemType Directory -Path $DestinationDirectory -Force | Out-Null
$stamp = Get-Date -Format "yyyyMMdd-HHmmss"
$staging = Join-Path ([IO.Path]::GetTempPath()) "occluview-preview-diagnostics-$stamp"
$archive = Join-Path $DestinationDirectory "OccluView-preview-diagnostics-$stamp.zip"

try {
    New-Item -ItemType Directory -Path $staging -Force | Out-Null
    Copy-Item -LiteralPath $source -Destination (Join-Path $staging "preview-failures.jsonl") -Force
    Compress-Archive -LiteralPath (Join-Path $staging "preview-failures.jsonl") -DestinationPath $archive -Force
} finally {
    Remove-Item -LiteralPath $staging -Recurse -Force -ErrorAction SilentlyContinue
}

Write-Host "Collected OccluView preview diagnostics: $archive"
