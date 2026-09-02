<p align="center">
  <img src="assets/occluview-logo.png" width="84" height="84" alt="OccluView logo">
</p>

<h1 align="center">OccluView</h1>

<p align="center">Native 3D viewing and mesh editing for digital dental scans.</p>

<p align="center">
  <a href="https://github.com/occlutrace/OccluView/releases/latest"><strong>Download for Windows</strong></a>
  &nbsp;·&nbsp;
  <a href="https://github.com/occlutrace/OccluView/actions/workflows/ci.yml">Build status</a>
</p>

<p align="center">
  <img src="assets/screenshot1.png" width="820" alt="OccluView open with a dental scan">
</p>

| Platform | Package | Includes |
| --- | --- | --- |
| Windows | `OccluView-Windows-Setup.msi` | Viewer, associations, thumbnails, and Explorer Preview Pane. |
| Windows | `OccluView-Windows-Portable.zip` | Viewer only; no Explorer integration. |
| Debian/Ubuntu | `OccluView-Linux.deb` | Viewer, launcher, MIME setup, and thumbnails. |

OccluView opens STL, PLY, OBJ, GLB, and HPS; it exports STL, PLY, and OBJ.

- `.hps` and `.dcm` are accepted for dental HPS containers; medical DICOM is not supported (`DICM` is refused), and `.dcm` is never the default file association.

## Controls

- Open a scan with **Ctrl+O**. Opening another file adds a layer; toolbar Open replaces the scene.
- Toolbar tools: **C** cut view, **M** ruler, **T** thickness, **A** align scans, **E** edit mesh.
- Orbit with right-drag; pan with middle-drag; zoom with the wheel; recenter on a surface with middle-click or double-click.
- In Mesh Editor: **Ctrl+A** selects; **Delete** or **Backspace** removes; **Ctrl+Z**, **Ctrl+Y**, or **Ctrl+Shift+Z** undo and redo; **Enter** closes an outline; **Esc** cancels it.
- In Sculpt: **1** chooses Add/Remove, **2** chooses Smooth, and holding **Shift** inverts or strengthens the active brush.
- **Ctrl+Middle-click** hides a layer; **Ctrl+Shift+Middle-click** restores the last hidden layer; **Shift+Middle-click** toggles translucency.

## The cut view

Plant the disc on a surface, then drag it to position the section. Plain wheel
zooms the panel; **Ctrl+wheel** changes disc size. **F** flips the kept half
while the disc is planted. **Esc** unplants the disc or closes Cut View.

## Windows Explorer

The MSI registers thumbnails and an interactive Preview Pane. In that pane,
right-drag orbits, wheel zooms, **F** frames the model, and **W** toggles
wireframe.

## Command line

```text
occluview-cli thumbnail <file> [-o out.png] [--size N]
occluview-cli convert <file> -o output.{stl|ply|obj}
occluview-cli close-holes <file> -o out.stl [--limit-mm N]
occluview-cli info <file> [file...]
```

`thumbnail` uses the same rendering path as Explorer and Linux file managers.

## Build

```bash
cargo fmt --all --check
cargo test --workspace --all-targets --locked
```

Security reporting and update-signing details are in [SECURITY.md](SECURITY.md).
Contributions are covered by [CONTRIBUTING.md](CONTRIBUTING.md). Licensed under
[Apache-2.0](LICENSE); distribution notices are in [NOTICE](NOTICE) and
[third-party notices](THIRD-PARTY-NOTICES.md).
