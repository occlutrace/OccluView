<p align="center">
  <img src="assets/occluview-logo.png" width="88" height="88" alt="OccluView logo">
</p>

<h1 align="center">OccluView</h1>

<p align="center">
  Native 3D viewer and lightweight mesh editor for dental scans.
</p>

<p align="center">
  <img src="assets/screenshot1.png" alt="OccluView showing a dental scan" width="820">
</p>

OccluView is an open-source desktop viewer from occlutrace.ai for
dental scans and common mesh files.

## Features

- Full-window orthographic 3D viewport with orbit, pan, retargeting, cut view,
  ruler, and wall-thickness measurement.
- Multiple layers with visibility, opacity, tint, and wireframe controls.
- Mesh editing: delete, crop, cut, separate, close selected holes, keep the
  largest component, invert normals, repair, smooth, and undo/redo.
- Windows Explorer thumbnails, live Preview Pane, file associations, and
  context-menu integration installed by the MSI.
- Native Linux desktop integration with MIME registration and a thumbnailer.

<p align="center">
  <img src="assets/animation.gif" alt="OccluView live Windows Preview Pane" width="820">
</p>

## Formats

| Format | Support |
| --- | --- |
| `.stl` | Binary and ASCII mesh |
| `.ply` | Binary and ASCII mesh with vertex colors |
| `.obj` | Mesh and vertex colors |
| `.glb` | Meshes with embedded textures supported by the viewer |
| `.hps` | 3Shape dental HPS containers |

## Windows

The MSI installer registers supported mesh formats and installs the Explorer
thumbnail and Preview Pane handlers. Opening another file while OccluView is
already running adds it to the current scene instead of opening another window.
File types use neutral names and one shared 3D-object icon.

## Linux

The Debian package installs the application, desktop entry, MIME registration,
icon, and thumbnailer.

## Performance

Measured end to end on a Ryzen 5 3600 with a warm file cache:

| File | Size | Open time |
| --- | --- | --- |
| Intraoral scan, 300k triangles | 15 MB binary STL | about 0.1 s |
| 1M triangles | 50 MB binary STL | about 0.4 s |
| 5M triangles | 250 MB binary STL | about 1.8 s |

Results depend on storage, CPU, mesh topology, and texture data.

## Download

The latest release (https://github.com/occlutrace/OccluView/releases/tag/v1.0.2).

- **Windows MSI**: normal installation with Explorer integration.
- **Windows portable ZIP**: runs without installation or Explorer integration.
- **Debian package**: for Debian, Ubuntu, and compatible desktops.

```bash
sudo apt install ./occluview_<version>_amd64.deb
```

Release files include SHA-256 checksums.

## Build from source

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
cargo run -p occluview-app --release -- path/to/scan.stl
```

Generate a thumbnail with the CLI:

```bash
cargo run -p occluview-cli --release -- thumbnail scan.stl -o thumb.png --size 256
```

```bash
cargo run -p occluview-cli --bin occluview-hps-export --release -- \
  --input scan.hps \
  --output-dir ./converted
```


## License

Apache-2.0. See [LICENSE](LICENSE).
