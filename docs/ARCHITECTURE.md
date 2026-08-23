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

- **File parsers** (STL/PLY/OBJ/GLB/HPS/OFF) handle untrusted bytes from Explorer/thumbnail. OFF is on this boundary even though it is not a user-facing format: `probe()` matches magic before extension, so any file whose header starts with `OFF` reaches `off::read` regardless of its name. Bounded by ZIP entry 256 MiB, aggregate 512 MiB, texture 8192 px edge with a 256 MiB decoded-RGBA ceiling -- one budget shared by every reader, defined once in `occluview-hps` -- checked arithmetic.
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
- **Reproducibility**: the generated embedded-key module is derived from the key bytes and `OCCLUVIEW_HPS_KEY_SALT` (default: the crate version) and from nothing else, so the same source, key, salt and toolchain rebuild to the same bytes. That is the boundary this repository controls; the compiler, the linker and the C++ CSG kernel's own build are not audited for bit-for-bit reproducibility, so this is a "no deliberate entropy" guarantee rather than a full one.

## Fuzzing

`fuzz/` crate with `cargo-fuzz` targets: `dispatch` (all formats), `stl`, `ply`, `glb`, `hps_parser`. `glb` is separate because it is the one supported format carrying an attacker-chosen offset table, and mutations sharing a budget with eleven other readers rarely reach it. Each run starts from the tracked seeds in `fuzz/seeds/` plus `fuzz/dictionaries/formats.dict`, and writes into `fuzz/corpus/`, which CI caches so a deep run feeds the next. CI runs a 60s smoke on every push and a 300s deep fuzz weekly on `schedule: 0 2 * * 1`. Locally: `./scripts/run-fuzz.sh <target> 300 131072` from the repository root.
