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
