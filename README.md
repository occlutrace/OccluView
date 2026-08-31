<p align="center">
  <img src="assets/occluview-logo.png" width="72" height="72" alt="OccluView logo">
</p>

<h1 align="center">OccluView</h1>

<p align="center">Native desktop viewer and mesh editor for dental 3D scans.</p>

<p align="center">
  <a href="https://github.com/occlutrace/OccluView/actions/workflows/ci.yml"><img src="https://github.com/occlutrace/OccluView/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/occlutrace/OccluView/releases/latest"><img src="https://img.shields.io/github/v/release/occlutrace/OccluView?label=release" alt="Latest release"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0-blue" alt="Apache-2.0 license"></a>
</p>

## Download

[Get the latest release](https://github.com/occlutrace/OccluView/releases/latest).
Published releases provide a Windows MSI, a portable Windows ZIP, and a Debian
package. Installable packages carry SHA-256 checksums, minisign signatures, and
build provenance; see the release page for the matching files.

## What it does

- **View and inspect** — orthographic 3D viewport, layers, cut view, ruler,
  and thickness measurement.
- **Edit** — click, box, and lasso selection; crop, cut, separate, repair,
  hole closing, sculpting, and undo/redo.
- **Compare** — align two scans and inspect their deviation heatmap.
- **Integrate** — use the desktop viewer or `occluview-cli`; receive Explorer
  thumbnails and Preview Pane support on Windows, or freedesktop thumbnails on
  Linux.

## Files and platforms

| Input | Export | Packages |
| --- | --- | --- |
| STL, PLY, OBJ, GLB, HPS | STL, PLY, OBJ | Windows 10+ x86_64 (MSI or ZIP); Debian x86_64 |

Dental HPS containers may use the `.dcm` extension. Medical DICOM is detected
and refused; the installer does not claim `.dcm` as a default association.
Details on encrypted HPS files and the optional source-build key are in the
[usage guide](docs/USAGE.md).

## Documentation

- [Using OccluView](docs/USAGE.md) — controls, editing tools, cut view,
  alignment, and CLI commands.
- [Architecture](docs/ARCHITECTURE.md) — workspace boundaries, rendering, file
  handling, shell integration, and update verification.
- [Contributing](CONTRIBUTING.md) — local build, test, and fuzzing workflow.
- [Security policy](SECURITY.md) — private vulnerability reporting.

## License

OccluView is licensed under [Apache-2.0](LICENSE). Distribution notices are in
[NOTICE](NOTICE), [third-party Rust notices](THIRD-PARTY-NOTICES.md), and
[native dependency notices](THIRD-PARTY-NOTICES-NATIVE.md).
