# OccluView

**A fast native Windows workspace for opening, comparing, and editing dental 3D scans.**

[Download for Windows](https://github.com/occlutrace/OccluView/releases/latest) · [All releases](https://github.com/occlutrace/OccluView/releases)

<p align="center">
  <img src="assets/occluview-workspace.png" width="100%" alt="OccluView Windows workspace with a dental scan and editing tools">
</p>

## Work with scans

- **Align** — position two scans and inspect their deviation on a color map with a millimeter scale.
- **Sculpt** — add, remove, or smooth local surface detail with adjustable brushes.
- **Bridge Split** — position a separator disc, preview the split, and apply it to a bridge.
- **Scenes** — keep two independent workspaces open and move scan layers between them.
- **Mesh Editing and Repair** — select, cut, crop, separate, and repair scan geometry; close safe pinholes.
- **Windows Explorer** — the installer adds scan thumbnails and an interactive Preview Pane.

<details>
<summary>Sculpt demo</summary>
<p><img src="assets/sculpt-demo.gif" width="100%" alt="Sculpting a dental scan with the Smooth brush"></p>
</details>

<details>
<summary>Scan alignment demo</summary>
<p><img src="assets/align-demo.gif" width="100%" alt="Placing matching points on two dental scans"></p>
</details>

<details>
<summary>Bridge Split demo</summary>
<p><img src="assets/bridge-split-demo.gif" width="100%" alt="Positioning and applying a Bridge Split"></p>
</details>

<details>
<summary>Two-scene workflow</summary>
<p><img src="assets/scenes-demo.gif" width="100%" alt="Working with two independent OccluView scenes"></p>
</details>

<details>
<summary>Windows Explorer preview</summary>
<p><img src="assets/explorer-preview.png" width="100%" alt="Dental scan thumbnails and an interactive Windows Explorer preview"></p>
</details>

## Download

- [Windows Installer (.msi)](https://github.com/occlutrace/OccluView/releases/latest/download/OccluView-Windows-Setup.msi) — recommended; installs OccluView and Windows Explorer integration.
- [Windows Portable (.zip)](https://github.com/occlutrace/OccluView/releases/latest/download/OccluView-Windows-Portable.zip) — run without installing; does not add Explorer integration.

## File formats

Open STL, PLY, OBJ, GLB, and OFF files. Export meshes as STL, PLY, or OBJ. Textured PLY and OBJ files can use image textures stored alongside the model.


See [measurement definitions and unit decisions](docs/measurements.md) before
comparing readings from differently sampled meshes or ambiguous GLB exports.

## Project

- [Releases and downloads](https://github.com/occlutrace/OccluView/releases)
- [Report a problem](https://github.com/occlutrace/OccluView/issues)
- [Contributing](CONTRIBUTING.md)
- [License](LICENSE) — Apache-2.0
