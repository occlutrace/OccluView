<p align="center">
  <img src="assets/occluview-logo.png" width="84" height="84" alt="OccluView logo">
</p>

<h1 align="center">OccluView</h1>

<p align="center">A native 3D viewer and mesh editor for digital dental scans.</p>

<p align="center">
  <a href="https://github.com/occlutrace/OccluView/releases/latest"><strong>Download for Windows</strong></a>
  &nbsp;·&nbsp;
  <a href="docs/USAGE.md">Open the guide</a>
  &nbsp;·&nbsp;
  <a href="https://github.com/occlutrace/OccluView/actions/workflows/ci.yml">Build status</a>
</p>

<p align="center">
  <img src="assets/screenshot1.png" width="820" alt="OccluView open with a dental scan">
</p>

## Install once, then work normally

| Your workstation | Download on the release page | What it gives you |
| --- | --- | --- |
| **Windows — recommended** | `OccluView-Windows-Setup.msi` | The viewer, file thumbnails, and Explorer Preview Pane. Run this installer once. |
| **Windows — no installation** | `OccluView-Windows-Portable.zip` | Open the viewer manually. It does not add Explorer previews or file associations. |
| **Linux (Debian/Ubuntu)** | `OccluView-Linux.deb` | The native viewer, launcher, MIME setup, and thumbnails. |

For a Windows lab, choose the **MSI installer**. The other files are for a
portable copy or a Linux workstation.

## From a folder to a usable scan

<p align="center">
  <img src="assets/explorer-preview.gif" width="640" alt="Explorer thumbnails and live 3D preview for dental scans">
</p>

- Inspect a scan in orthographic 3D, use layers, Cut View, ruler, and thickness measurement.
- Select, crop, cut, separate, repair, close holes, sculpt, and undo work safely.
- Align two scans and review a deviation heatmap.

## Files

| Open | Export |
| --- | --- |
| STL, PLY, OBJ, GLB, HPS | STL, PLY, OBJ |

- `.hps` and `.dcm` are accepted for dental HPS containers; medical DICOM is not supported (`DICM` is refused), and `.dcm` is not a default association.
  Details on encrypted HPS files and the optional source-build key are in the [usage guide](docs/USAGE.md).

<details>
<summary>For clinic IT and verification</summary>

Normal users only need the installer. Releases retain checksums, signatures,
build provenance, and SBOMs for teams that need to verify or audit a package.

</details>

## More

[Using OccluView](docs/USAGE.md) · [Architecture](docs/ARCHITECTURE.md) · [Security policy](SECURITY.md) · [Contributing](CONTRIBUTING.md)

Licensed under [Apache-2.0](LICENSE). Distribution notices are in [NOTICE](NOTICE) and the [third-party notices](THIRD-PARTY-NOTICES.md).
