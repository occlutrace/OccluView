OccluView Preview Diagnostics

This non-release package is for developer support, not a normal installer.

1. As the affected Windows user, run Enable-PreviewDiagnostics.ps1. It needs no admin rights.
2. Reproduce the Preview Pane problem.
3. Run Collect-PreviewDiagnostics.ps1 -DestinationDirectory "$env:USERPROFILE\Desktop".

The collected archive contains only fixed diagnostic JSONL fields: no source mesh/path,
file name, raw error text, or scan content. No dumps are collected automatically.
