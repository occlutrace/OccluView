# Architecture

## Workspace layering

> See [`Cargo.toml:3-14`](../Cargo.toml#L3-L14) for the canonical layering comment.

```
mesh-edit  ← (nothing)
hps        ← (no OccluView crates)
align      ← (nothing; plain slices in, plain values out)
core       → mesh-edit (+ optional robust-csg)
formats    → core + hps
render     → core
thumbnail  → core + render + formats
shell      → core + render + formats + thumbnail
app        → core + formats + render + align + update (+ shell on Windows)
cli        → core + formats + render + thumbnail
update     → (nothing; minisign / semver / ureq)
```

Cycles are P0. `publish = false` — not on crates.io.

## Crate responsibilities

| Crate | Role |
|-------|------|
| `occlu-mesh-edit` | Pure kernels: holes, brush, bridge-split, repair, topology. `forbid(unsafe)`. |
| `occluview-robust-csg` | FFI boundary for `manifold-csg`. Optional feature in `core`. |
| `occluview-align` | Rigid ICP + deviation, slices in / values out. No IO/GPU. |
| `occluview-core` | Scene, mesh, camera, bbox. Units mm, Y-up, RH. Owns domain model. |
| `occluview-hps` | HPS/XML parsing, decryption, texture decode. Private `private-hps-key` feature. |
| `occluview-formats` | Readers: STL/PLY/OBJ/glTF/HPS. Single `memmap2` unsafe site. |
| `occluview-render` | wgpu pipeline, `PreparedScene`. One pipeline shared by app and thumbnail. |
| `occluview-thumbnail` | Platform-neutral thumbnail with bounded concurrency and placeholder ladder. |
| `occluview-shell` | Windows COM: thumbnail, preview handler, file associations. `release-unwind`. |
| `occluview-update` | Signed `latest.json` manifest, minisign verification. |
| `occluview-app` | Desktop GUI (egui/eframe). Largest crate — state, overlays, workers. |
| `occluview-cli` | Headless thumbnail/export, no shell dependency. |

## Trust boundaries

- **File parsers** (STL/PLY/OBJ/GLB/HPS/OFF) handle untrusted bytes from Explorer/thumbnail. OFF is on this boundary even though it is not a user-facing format: `probe()` matches magic before extension, so any file whose header starts with `OFF` reaches `off::read` regardless of its name. Bounded by ZIP entry 256 MiB, aggregate 512 MiB, texture 8192 px with an RGBA ceiling of 256 MiB for HPS and 64 MiB for glTF, checked arithmetic.
- **Thumbnail** runs in `dllhost.exe`. Panics unwind via `release-unwind`; `catch_unwind` substitutes a placeholder. One renderer, 12 job slots, per-request timeout.
- **Updater** verifies HTTPS, SHA-256, and minisign signature before installing. It is also the only outbound network call the product makes: two GETs per launch for `latest.json` and its signature, offer-only, never a silent install. `OCCLUVIEW_NO_UPDATE_CHECK` (any value) disables it; README documents this for packagers.
- **HPS key**: embedded key is obfuscation, not a secret boundary. Documented as friction; real entitlement should be per-device.

## Rendering

One `occluview-render` pipeline: one shader, one camera. A given mesh therefore rasterizes identically in Explorer and in the app. Above the per-format fidelity cutoffs in `render_thumb/loading.rs` the tile is drawn from a decimated preview mesh through that same pipeline, so it matches the shading but not the geometry. Orthographic camera, `GpuCamera` / `GpuMeshUniform` per mesh.

## Build and release

- `cargo fmt`, `cargo clippy -- -D warnings`, `cargo test --workspace`, `cargo doc --no-deps --all-features -D warnings`, `cargo deny` in CI.
- `rust-toolchain.toml` pins `1.86.0`.
- `install/build-msi.ps1` builds `occluview-app` (release) and `occluview-shell` (release-unwind), signs via `signtool.exe` (certstore or PFX), runs WiX v3, signs MSI. Tagged releases require Authenticode.
- `install/linux/build-deb.sh` + validation via `desktop-file-validate` / `appstreamcli` / `lintian`.
- Publish verifies `Cargo.toml` version == tag, smoke-installs MSI, signs artifacts with minisign, writes `latest.json`, uploads to GitHub Release.

## Fuzzing

`fuzz/` crate with `cargo-fuzz` targets: `dispatch` (all formats), `hps_parser`, `stl`, `ply`. CI runs a 60s smoke over the 4 targets on every push, and a 300s deep fuzz weekly on `schedule: 0 2 * * 1` (Mondays, 02:00 UTC). For deeper runs: `cargo fuzz run <target> -- -max_total_time=300` in `fuzz/`.
