# Security

Please report security issues privately: **security@occlutrace.ai**.

Useful reports include:

- the file or steps needed to reproduce the issue;
- the expected and actual behaviour;
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

The Package workflow builds the Apple Silicon `.app`, `.dmg`, and `.pkg` with
the embedded HPS key. A macOS asset is published, and advertised by the update
manifest, only when the build was signed with Developer ID Application (app and
disk image) and Developer ID Installer (package), notarized, and stapled; the
update assets still carry minisign signatures like every other platform. The
signing step runs when all of these release secrets are set, and refuses a
partial set:

- `OCCLUVIEW_MACOS_APP_P12_BASE64` and `OCCLUVIEW_MACOS_INSTALLER_P12_BASE64`:
  the two Developer ID certificates with their private keys, as base64 PKCS#12;
- `OCCLUVIEW_MACOS_P12_PASSWORD`: the password of both files;
- `OCCLUVIEW_MACOS_APP_IDENTITY` and `OCCLUVIEW_MACOS_INSTALLER_IDENTITY`: the
  signing identity names, e.g. `Developer ID Application: <name> (<team>)`;
- `OCCLUVIEW_NOTARY_KEY_P8_BASE64`, `OCCLUVIEW_NOTARY_KEY_ID`, and
  `OCCLUVIEW_NOTARY_ISSUER`: an App Store Connect API key for `notarytool`.

Without them the packages stay unsigned workflow artifacts and the release
carries no macOS asset. No Apple signing credentials are stored in this
repository.

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
