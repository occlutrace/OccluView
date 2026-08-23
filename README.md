<p align="center">
  <img src="assets/occluview-logo.png" width="88" height="88" alt="OccluView logo">
</p>

<h1 align="center">OccluView</h1>

<p align="center">A fast, native 3D viewer for dental scans and files with basic mesh editing and sculpting features.</p>

<p align="center">
  <a href="https://github.com/occlutrace/OccluView/actions/workflows/ci.yml"><img src="https://github.com/occlutrace/OccluView/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/occlutrace/OccluView/releases/latest"><img src="https://img.shields.io/github/v/release/occlutrace/OccluView?label=release" alt="Latest release"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0-blue" alt="License"></a>
</p>

<p align="center">
  <img src="assets/screenshot1.png" alt="OccluView showing a dental scan" width="820">
</p>

OccluView was created with a "nothing extra" philosophy. It offers lightning-fast file opening, mesh editing, surface sculpting, and basic scan alignment functions, complete with a discrepancy heatmap.
The viewer is tailored to the daily work of CAD designers. It supports multiple layers, a surface ruler, a thickness analyzer, and a truly convenient cut view. It also integrates with Windows Explorer and has its own Linux package.

## Features

- Orthographic 3D viewport with orbit, pan, cut view, ruler, and thickness measurement.
- Multiple layers with visibility, opacity, tint, and wireframe controls.
- Selection by click, rectangle, or lasso.
- Mesh editing with delete, crop, cut, separate, close holes, repair, smooth, and undo/redo.
- Scan alignment with a discrepancy heatmap.
- Export back to STL, PLY, or OBJ — the whole scene or one layer at a time, so edits leave the app.
- `occluview-cli`, a headless companion for thumbnails, conversion, hole closing, and file info.
- Windows thumbnails, Preview Pane, file associations, and context-menu integration.
- Linux desktop integration with MIME registration and a thumbnailer.

<p align="center">
  <video src="assets/animation.webm" autoplay loop muted playsinline width="820"></video>
  <img src="assets/animation.gif" alt="OccluView live Windows Preview Pane" width="820">
</p>

## Supported formats

- `.stl` - binary and ASCII meshes
- `.ply` - binary and ASCII meshes with vertex colors
- `.obj` - meshes and vertex colors
- `.glb` - meshes with embedded textures
- `.hps` and `.dcm` - 3Shape/HPS dental containers that use the `.dcm` extension (medical DICOM is not supported; files with a `DICM` marker at offset 128 are rejected)

Encrypted HPS/CE containers need a decryption key. Official builds embed one;
a build from source reads `OCCLUVIEW_HPS_ENCRYPTION_KEY` from the environment,
and without either the file is reported as encrypted rather than opened.

`.off` meshes are read too, recognised by their header rather than their
extension. They are not offered in the open dialog and not registered with the
desktop, so open them by dropping the file onto the window.

`.dcm` is shared with medical DICOM, so neither installer claims it. OccluView
is offered for `.dcm` in *Open with* and in the system's default-application
list, and the extension keeps whichever viewer already owns it — on a
workstation that also holds CBCT data, nothing changes hands.

## Download and verification

[Download the latest release](https://github.com/occlutrace/OccluView/releases/latest)

The release page contains the Windows installer, portable Windows archive, Debian package, SHA-256 checksums, and minisign signatures. See `docs/ARCHITECTURE.md` for the update verification model (`latest.json` with per-artifact signatures).

Windows installers and binaries are Authenticode-signed when a certificate is configured; tagged releases require a valid signature.

### Verify your download

Every release asset ships a SHA-256 file and a minisign signature, and the
release itself carries a GitHub build-provenance attestation and a CycloneDX
SBOM per platform. The signing public key is [`occluview.pub`](occluview.pub)
in this repository — the same key compiled into the updater:

```
RWRoIIL40qxwrFOI5OeCx0Fcf1ClUksy36PrIZrdKkGhQq2kFOtITQnq
```

Run all three from the directory you downloaded into:

```bash
# 1. checksum
sha256sum -c occluview_1.0.9_amd64.deb.sha256

# 2. signature, against the key above
minisign -Vm occluview_1.0.9_amd64.deb -P RWRoIIL40qxwrFOI5OeCx0Fcf1ClUksy36PrIZrdKkGhQq2kFOtITQnq

# 3. provenance: this artifact was built by this repository's release workflow
gh attestation verify occluview_1.0.9_amd64.deb --repo occlutrace/OccluView
```

The same three commands work for the MSI and the portable ZIP. `sbom-windows.json`
and `sbom-linux.json` on the release page list every third-party component that
went into the binaries, for anyone whose scanner wants it.

### Update check

The viewer asks once per launch whether a newer release exists: two HTTPS GETs
for `latest.json` and its signature from this repository's releases page. It
sends nothing beyond an ordinary HTTP request, it never installs anything on its
own, and the only thing it stores is a local marker when a version is dismissed.
The manifest is checked against the key above before anything is downloaded, and
an accepted update is checked again — SHA-256 and signature — before it runs.

To turn it off entirely — packagers, managed clinic images — set
`OCCLUVIEW_NO_UPDATE_CHECK` in the environment. Any value will do.

## Windows

The MSI installs the viewer together with Explorer thumbnails, Preview Pane support, file associations, and the shared 3D-object icon. Opening another file while the viewer is running adds it to the current scene.

## Linux

The Debian package installs the viewer, the `occluview-cli` companion, the
desktop entry, MIME registration, icon, and thumbnailer. Both binaries land on
`PATH`; `occluview-cli --help` lists the headless commands.

## Architecture

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for workspace layering, crate responsibilities, trust boundaries, rendering, and the build/release pipeline.

## Supported platforms

- Windows 10+ (x86_64, MSI + portable ZIP)
- Linux (x86_64, Debian package; freedesktop thumbnailer)

## Build from source

### Prerequisites

The workspace links a native CSG kernel (Manifold), whose build script clones
its C++ sources and builds them with CMake. That makes a C++ toolchain, CMake,
git **and network access** hard requirements for a first build — without them
the very first `cargo` command fails inside a transitive `-sys` crate rather
than in this workspace.

- **Linux**: `build-essential cmake git`, plus a Vulkan driver for the renderer
  tests (`mesa-vulkan-drivers` covers headless machines through Lavapipe). The
  runtime libraries the `.deb` depends on are listed in
  `install/linux/build-deb.sh`.
- **Windows**: Visual Studio Build Tools with the C++ workload, CMake, and git.
  WiX Toolset 3.14 is needed only to build the MSI.
- **Offline builds**: set `MANIFOLD_CSG_LIB_DIR` to a prebuilt Manifold and the
  clone and CMake steps are skipped.

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
cargo run -p occluview-app --release -- path/to/scan.stl
```

Fuzz targets: `cargo fuzz run <target> -- -max_total_time=60` in `fuzz/` (requires `cargo install cargo-fuzz`); weekly deep fuzz 300s on schedule (Mondays, 02:00 UTC).

## Security

See [SECURITY.md](SECURITY.md). Report vulnerabilities privately to **security@occlutrace.ai**.

## License

Apache-2.0. See [LICENSE](LICENSE) and [NOTICE](NOTICE). Third-party crates
and bundled fonts are listed with their license texts in
[THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md), which every installer and
package also ships.

---

OccluTrace
