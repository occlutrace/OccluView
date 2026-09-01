[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

# This switch is intentionally per-user. The diagnostic MSI only makes this
# helper available; it never enables logging while running elevated.
$diagnosticKey = "HKCU:\Software\OccluTrace\OccluView\Diagnostics"
New-Item -Path $diagnosticKey -Force | Out-Null
New-ItemProperty `
    -Path $diagnosticKey `
    -Name "ShellEventLogEnabled" `
    -PropertyType DWord `
    -Value 1 `
    -Force | Out-Null

Write-Host "OccluView shell diagnostics are enabled for the current Windows user."
Write-Host "Logs contain fixed lifecycle categories only and are written by the optional diagnostic package."
