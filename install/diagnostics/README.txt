OccluView Shell Diagnostics

This non-release package is for developer support, not a normal installer.

1. As the affected Windows user, run Enable-PreviewDiagnostics.ps1. It needs no admin rights.
2. Reproduce the Preview Pane or thumbnail problem.
3. Run Collect-PreviewDiagnostics.ps1 -DestinationDirectory "$env:USERPROFILE\Desktop".

The collected archive contains fixed diagnostic JSONL fields plus a narrow snapshot of
OccluView's own Shell registration entries. It contains no source mesh/path, file name,
raw error text, scan content, GPU driver details, or automatic dumps.
No dumps are collected automatically.
