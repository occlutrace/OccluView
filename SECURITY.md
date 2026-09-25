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
against the manifest's SHA-256 and its own signature before the OS receives it.
On macOS, the verified `.pkg` is opened with LaunchServices; Installer presents
the package and the operator authorizes changes to `/Applications`. Nothing
runs with elevated privileges from inside OccluView. The only state kept is a
local marker for a dismissed version.

Set `OCCLUVIEW_NO_UPDATE_CHECK` (any value) to disable the check entirely.
`occluview-update` is the only crate with an HTTP client; nothing else in the
workspace reaches the network. The single-instance handshake uses a local Unix
socket on Linux, a named pipe on Windows, and a kernel-managed file lock plus
local state-directory handoff on macOS; it never leaves the machine.

## Local state

The viewer writes one state directory: `%APPDATA%\OccluView\` on Windows,
`$XDG_STATE_HOME/OccluView/` (falling back to `~/.local/state/OccluView/`) on
Linux, or `~/Library/Application Support/OccluView/` on macOS. It contains
recent-file paths, crash reports, the skipped-update marker, and short-lived
hand-off files. Crash reports deliberately omit scan paths.

## Signing keys

Release secrets are stored only in the release system, never in this
repository. To rotate the update key, first ship a release that trusts the new
public key, then sign subsequent releases with the new private key. Remove the
old key only after installed versions can verify the replacement.

## macOS distribution gate

Local Apple Silicon `.app`, `.dmg`, and `.pkg` outputs are unsigned developer
artifacts and are not published. Before a macOS release is added, a maintainer
must sign the app with Developer ID Application, sign its installer package
with Developer ID Installer, notarize and staple the distributed artifacts,
and still attach the existing minisign signatures to update assets. No Apple
signing credentials are stored in this repository. Until that gate is met, no
macOS asset is advertised by the release or update manifest.

## Verifying a release

Every release asset carries SHA-256 and minisign signatures; releases also
carry GitHub build provenance and a CycloneDX SBOM.

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
