<div align="center">

# OccluView

**A lightning-fast, low-RAM, native Windows 3D mesh viewer for dental workflows.**

Open STL · OBJ · PLY · glTF/GLB · 3MF — with live 3D thumbnails right inside
Windows Explorer.

[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![Build](https://img.shields.io/badge/build-Cargo-orange.svg)](https://www.rust-lang.org/)
[![Windows](https://img.shields.io/badge/platform-Windows%2010/11-0078D4)](https://occlutrace.ai)

</div>

---

OccluView is an open-source desktop viewer built by
[OccluTrace](https://occlutrace.ai) for dental labs and clinics.

> **Why now?** Microsoft deprecated 3D Viewer in February 2026 and removed it
> from the Store on **July 1, 2026** — and it never provided STL/PLY thumbnails
> anyway. Paint 3D, 3D Builder, and Print 3D are already gone. There is no
> first-party answer for opening dental scans on Windows. OccluView fills that
> gap.

It is designed around three principles:

1. **It just opens.** Cold start under 400 ms, idle RAM under 120 MB. It is an
   *opener*, not an editor. Drop a file, see it, move on.
2. **It belongs in Windows.** Native 3D thumbnails in Explorer, default
   "Open with", jumplist of recent cases — the things that make a tool feel
   built-in, not bolted-on.
3. **It speaks dental.** Millimeter units, occlusal default camera, upper/lower
   arch pairing, vertex-color scan support. Not a generic game-mesh viewer with a
   tooth icon.

## Project status

🚧 **Foundational.** Architecture and governance are in place; implementation is
getting started. See [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) and the
[open issues](../../issues) for the roadmap.

## Supported formats

| Format | Binary | ASCII | Vertex colors | Materials | Status                          |
|--------|:------:|:-----:|:-------------:|:---------:|---------------------------------|
| STL    | done   | done  | —             | —         | **shipped** (binary + ASCII)    |
| PLY    | done   | done  | done          | —         | **shipped** (LE/BE/ASCII, color)|
| OBJ    | —      | todo  | via `mtl`     | todo      | next                            |
| glTF/GLB | todo | todo  | todo          | todo (PBR)| via cgltf                       |
| 3MF    | todo   | —     | todo          | todo      | via lib3mf (BSD-3)              |

PLY is the strategic dental format: it is the only one of these that carries
per-vertex color (NIRI/mucosal-shade scans), and recent peer-reviewed work
recommends it for long-term archival of intraoral scans. See
[`docs/FORMAT_SUPPORT.md`](docs/FORMAT_SUPPORT.md).

## Architecture in one paragraph

One Rust workspace, one language, one renderer. `occluview-core` (pure logic),
`occluview-formats`, `occluview-render` (wgpu), `occluview-shell` (Windows COM
thumbnail provider — same renderer, offscreen), and `occluview-app` (egui GUI).
The thumbnail provider and the app share the exact same mesh loader and shader
pipeline, so what you see in Explorer is what you get in the window. See
[`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md).

## Building from source

Requirements: Rust toolchain pinned in [`rust-toolchain.toml`](rust-toolchain.toml),
Windows 10/11 SDK.

```bash
git clone https://github.com/occlutrace/occluview
cd occluview
cargo build --workspace --release
```

Run the viewer:
```bash
cargo run -p occluview-app --release -- path/to/scan.stl
```

Generate a thumbnail headlessly:
```bash
cargo run -p occluview-cli --release -- thumbnail scan.stl -o thumb.png --size 256
```

## Contributing

OccluView is developed with AI coding agents under strict rules to keep the
codebase clean and honest. **Before opening a PR, read
[`AGENTS.md`](AGENTS.md)** — it is binding for every contributor, human or AI.
The short version: evidence over assertion, tests travel with code, one
responsibility per file, conventional commits, no AI slop.

Useful entry points:
- [`AGENTS.md`](AGENTS.md) — constitution + 7-stage workflow
- [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) — crate graph and data flow
- [`docs/adr/`](docs/adr/) — architecture decision records
- [`CONTRIBUTING.md`](CONTRIBUTING.md) — setup, conventions, Definition of Done

## Governance & licensing

- **License:** Apache-2.0 (see [`LICENSE`](LICENSE)). Open-core: the viewer is
  open; OccluTrace's cloud alignment service is a separate proprietary product.
- **DCO:** every commit signs off (`git commit -s`). See
  [`CONTRIBUTING.md`](CONTRIBUTING.md).
- **Trademark:** "OccluTrace" and "OccluView" are trademarks of OccluTrace, Inc.
  See [`TRADEMARK.md`](TRADEMARK.md).
- **Security:** see [`SECURITY.md`](SECURITY.md).

## Acknowledgements

OccluView stands on the shoulders of:
- [F3D](https://f3d.app) — the benchmark for fast minimalist 3D viewing.
- [stl-thumb](https://github.com/unlimitedbacon/stl-thumb) and
  [win-svg-thumbs-rust](https://github.com/ThioJoe/win-svg-thumbs-rust) — proof
  that Rust Windows thumbnail providers work.
- [wgpu](https://github.com/gfx-rs/wgpu), [egui](https://github.com/emilk/egui),
  [cgltf](https://github.com/jkuhlmann/cgltf),
  [fastgltf](https://github.com/spnda/fastgltf), [windows-rs](https://github.com/microsoft/windows-rs).

<div align="center">

Made with care for dental technicians by [OccluTrace](https://occlutrace.ai).

</div>
