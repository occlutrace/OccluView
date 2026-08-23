# Security

Please report security issues privately: **security@occlutrace.ai**.

Useful reports include:

- the file or steps needed to reproduce the issue;
- the expected and actual behavior;
- the affected version or commit.

Please do not open a public GitHub issue for a suspected vulnerability.

OccluView parses untrusted local files and has a Windows thumbnail provider, so
parser crashes, thumbnail hangs, installer problems, and dependency
vulnerabilities are all in scope.

## Network behaviour

The viewer makes exactly one kind of outbound request: on launch it fetches
`latest.json` and its minisign signature from this repository's releases, to
decide whether to offer an update. Nothing is sent beyond two ordinary HTTPS
GETs, nothing is installed without the operator choosing it, and the manifest is
verified against a public key compiled into the binary
([`occluview.pub`](occluview.pub)) before any download is offered. If the
operator accepts, one further GET fetches the installer, which is checked
against the manifest's SHA-256 and its own signature before it runs. The only
state kept is a local marker for a dismissed version.

Set `OCCLUVIEW_NO_UPDATE_CHECK` (any value) to disable the check entirely.
`occluview-update` is the only crate with an HTTP client; nothing else in the
workspace reaches the network. The single-instance handshake uses a local Unix
socket or named pipe and never leaves the machine.

## Local state

The viewer writes to one directory: `%APPDATA%\OccluView\` on Windows (the
roaming profile, so it follows a domain user between machines) or
`$XDG_STATE_HOME/OccluView/` on Linux. It holds the recent-files list, crash
reports, the skipped-update marker, and short-lived hand-off files. Scan paths
are case identifiers in dental work, so note that the recent list contains them
and roams; crash reports deliberately do not — the startup log records how many
files and of which formats, never which. The README section *What OccluView
stores on this machine* has the full inventory and how to clear it.

## Verifying a release

Every asset carries a SHA-256 file and a minisign signature, releases carry a
GitHub build-provenance attestation, and each platform ships a CycloneDX SBOM.
The README's *Verify your download* section has the exact commands.

## Supported versions

Only the latest tagged release is supported with security fixes.

## Disclosure process

- Reports are acknowledged within 5 business days.
- We aim to provide a fix or mitigation within 90 days and to coordinate disclosure with the reporter.
- Please allow us time to prepare a release before public disclosure.

## Severity model

We triage by impact on confidentiality, integrity, and availability, with
priority for Explorer thumbnail/preview (automatic file handling) and installer
trust boundaries. Dependency advisories are tracked via `cargo deny`.

## Safe harbor

Good-faith security research against this repository is welcomed. Do not exfiltrate data, disrupt services, or violate applicable law.
