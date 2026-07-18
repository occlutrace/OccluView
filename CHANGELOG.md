# Changelog

## 1.0.4 - 2026-07-18

- Fixed Close Holes leaving sharp spike artifacts where a large interpolated
  cap met the surrounding surface after a lasso cut, and raised the cap size
  this covers (#9).
- Added a per-layer toggle to hide scan colors/texture and show a flat
  neutral material instead, and a one-click Smooth action for a marked face
  selection (#10, #11).
- Fixed DCM/HPS scans whose embedded JPEG texture atlas had its chroma
  swapped at the source, decoding blue where gingiva/enamel should read warm
  — and tightened the correction so it only fires on a whole-texture bias,
  not a real localized blue material (anti-glare spray, bite-registration
  silicone) sitting next to normal tissue color (#12).
- Fixed the Bridge Split / Cut View cutting disc jittering as the cursor
  moved and defaulting to a longitudinal orientation along the scan: it now
  derives its orientation from the scan's own stable shape instead of a
  single triangle's local normal, landing transverse across the arch, and
  rotates smoothly to track the local curve as the cursor moves around the
  arch instead of staying fixed in one direction.

## 1.0.3 - 2026-07-15

- Made Export Layer open in the source file's folder and preselect its
  writable mesh format for STL, PLY, and OBJ sources.
- Kept derived layers usable with a scene-source folder fallback and an
  explicit PLY fallback for readable formats without a matching writer.
- Added export warnings for payloads that the selected mesh format cannot
  preserve, plus regression coverage for source and derived layers.

## 1.0.2 - 2026-07-15

- Made Bridge Split rebuild source normals on a private working copy, so
  corrupt or stale normal payloads no longer block clipping or poison the
  generated parts; positions, indices, and UVs remain strict geometry inputs.
- Made interactive Close Holes selection-only: an empty selection is a no-op,
  and only selected visible faces can qualify a boundary loop. Whole-mesh
  filling remains an explicit internal/CLI and repair-pipeline operation.
- Added regression coverage for selected-rim closure, hidden-layer isolation,
  empty-selection no-ops, and corrupt-normal Bridge Split inputs.

## 1.0.1 - 2026-07-15

- Fixed HPS/DCM files with large compressed texture atlases being rejected as
  oversized raw RGBA payloads before JPEG decoding.
- Kept compressed texture color decoding deterministic instead of guessing
  channel order from the image contents.
- Made Bridge Split continue with a bounded surface fallback for open dental
  scans and importer topology residue, while preserving closed-solid behavior
  for valid CAD meshes.
- Added a non-fatal result path for topology preflight failures so the source
  mesh remains unchanged when no usable split can be produced.

## 1.0.0 - 2026-07-15

- Declared the first stable release after the viewer, editor, HPS path,
  Windows Explorer integration, Linux packaging, and update channel reached a
  single release-tested baseline.
- Moved thumbnail loading, rendering, caching, and placeholder handling into a
  platform-neutral crate shared by the Windows shell adapter, Linux
  thumbnailer, and headless CLI.
- Kept the HPS parser as the single format leaf and shipped conversion as a
  second machine-facing binary in `occluview-cli` instead of coupling it to
  the desktop viewer.
- Added minisign coverage for the portable Windows archive alongside the MSI
  and Debian update artifacts.

## 0.1.39 - 2026-07-13

- Deduplicated copied mesh files in Explorer thumbnail bursts using a bounded
  content cache and single-flight render path.
- Separated twelve bounded decode lanes from one reusable GPU renderer, avoiding
  Windows driver contention while keeping mixed folders responsive.
- Fixed the release workflow's tracked-source key scan for current Git runners.

## 0.1.38 - 2026-07-13

- Matched the bounded thumbnail renderer to Explorer's twelve-request fan-out,
  preventing mixed folders from queueing long enough for Windows to cache
  generic format icons while still retaining a hard worker lifetime budget.

## 0.1.37 - 2026-07-13

- Restored fast mixed-folder Explorer thumbnail generation with a bounded
  four-to-twelve worker budget, so large folders no longer serialize behind
  two long-running renders while timed-out workers remain accounted for.

## 0.1.36 - 2026-07-13

- Hardened Bridge Split for dental meshes made from overlapping, touching, or
  separately indexed shells without discarding valid small geometry.
- Kept finite separator behavior strict: only the placed disc can affect a
  component, while impossible placements fail atomically without changing the
  source scene.
- Added coverage for cavities, reflected shells, microscopic components,
  remote arch geometry, and importer-degenerate faces.
- Kept mixed-folder Explorer thumbnail work bounded and the Windows viewer
  camera responsive in the release build.

## 0.1.35 - 2026-07-12

- Restored `Close holes` to Mesh Editor. With no face marks it repairs safe
  interior holes across every visible mesh layer in one atomic, undoable scene
  operation; hidden layers remain untouched. Marked faces still scope the
  repair, and the optional rim-perimeter limit is available again.
- Bridge Split now gives the finite separator disc priority for plain meshes,
  so a disc placed on a curved arch cannot silently behave like an infinite
  plane and cut a remote arm outside its footprint.
- Kept the native CSG boundary isolated behind a safe Rust crate and made its
  ownership/layering explicit in the workspace contracts.

## 0.1.34 - 2026-07-12

- Bridge Split now evaluates the separator's finite footprint, avoiding false
  diameter errors caused by distant parts of a curved dental arch.
- Cut View and Bridge Split share steadier separator placement, direct disc
  manipulation, editable Section measurements, and consistent close controls.
- Edit Mesh now selects and edits across all visible mesh layers in one atomic
  operation; hidden layers retain their geometry and selection state.

## 0.1.33 - 2026-07-12

- Bridge Split now retries the robust CSG path when the direct capper creates
  an invalid part, and stabilizes output only when conversion to viewer mesh
  precision would otherwise invalidate a closed result.

## 0.1.32 - 2026-07-11

- Bridge Split now falls back to a robust finite-disc CSG operation for closed
  plain meshes when the direct cutter cannot form clean caps around pathological
  but topology-bearing CAD facets. The result remains two closed parts with the
  requested kerf; normal scan paths stay on the fast cutter.

## 0.1.31 - 2026-07-11

- Bridge Split now normalizes common importer residue in an isolated working
  copy before cutting: redundant zero-area or duplicate faces, small holes,
  inconsistent winding, and removable debris no longer require a manual
  repair step when the result can be made into two closed parts.
- Healthy meshes retain the direct split path; source geometry and its
  materials remain untouched until the completed split is applied.

## 0.1.30 - 2026-07-11

- Reworked Bridge Split around the separator disc actually placed by the
  operator. A split now proceeds only when that disc spans the full kerf
  cross-section and can produce two closed parts with the requested gap.
- Added a live Section view during Bridge Split, with the same Lines/Mesh,
  measurement, pan, zoom, and disc-size controls used by Cut View.
- Replaced the generic split failure with specific guidance for missed,
  tangent, undersized, open, and invalid mesh cases.

## 0.1.13 - 2026-07-08

- Added public README media for the main viewer and the Windows Explorer live
  Preview Pane, so the repository shows the actual product instead of only
  install notes.
- Updated the README around the current Windows experience: MSI-installed file
  associations, Explorer thumbnails, one neutral 3D file icon, and interactive
  Preview Pane support for supported mesh formats.
- Continued the architecture cleanup by splitting large shell registration,
  glTF reader, core mesh, and preview-handler modules into focused internal
  files while preserving the public Rust APIs and Windows shell ABI.
- Hardened Linux single-instance file opens so a background viewer keeps waking
  until a file-manager handoff is consumed, then repeats the foreground pulse
  after the appended scene is ready.
- Kept the release path on the tag-driven Windows MSI / portable ZIP / Debian
  package workflow.

## 0.1.12 - 2026-07-08

- Continued the app architecture cleanup by moving startup bootstrap, scene
  loading, and dialog/chrome helper logic out of the main viewer file.
- Hardened single-instance open handoff with a bounded framed request format,
  legacy fallback parsing, and stricter path validation.
- Hardened Explorer thumbnail stream reuse by rewinding pending streams before
  lazy reads, adding offset-stream smoke coverage, and covering stream-cache
  eviction.
- Reduced duplicate Explorer thumbnail work during burst folders by coalescing
  concurrent identical requests after cache misses, while keeping bounded
  worker fan-out ahead of the renderer pool.
- Stopped timed-out followers in that in-flight thumbnail path from launching a
  second render under burst pressure; they now return the deterministic
  fallback instead of amplifying load in large mixed folders.
- Expanded the mixed-folder thumbnail smoke to cover larger burst folders and
  verify that non-3D neighbors do not turn into shell-path failures while real
  3D files still render actual thumbnails.
- Continued reducing brittle source-layout-sensitive tests by moving app/shell
  coverage toward the smaller modules that now own viewport, loading, and
  render behavior.
- Moved the main viewer render lifecycle and scene-state helpers out of
  `main.rs`, further shrinking the entrypoint into a thinner wiring layer.
- Quieted Linux shell-render test noise by setting an explicit runtime
  directory for GPU-backed shell tests.

## 0.1.11 - 2026-07-08

- Removed the visible layer overflow button and moved layer actions to the row
  right-click menu, while keeping the remove button inline.
- Corrected Explorer Preview Pane orbit input so drag direction matches the
  expected Windows preview feel without changing the main viewer camera.
- Hardened Explorer thumbnail bursts in mixed folders by deferring stream reads
  until `GetThumbnail`, rejecting unsupported noise before worker startup, and
  allowing a slightly wider bounded renderer pool.
- Kept OBJ thumbnail and preview fallback coverage for noisy small scanner
  files that the strict full parser may reject.

## 0.1.10 - 2026-07-08

- Hardened the Debian release path with an extracted-package smoke check that
  verifies required binaries, desktop integration files, MIME metadata,
  thumbnailer registration, maintainer scripts, XML/AppStream validity,
  shared-library resolution, and `lintian` errors in CI.
- Kept Windows and Linux release assets on one tag-driven workflow, with tag
  version checks before package publishing.
- Continued the app architecture cleanup by moving viewer helper, state-path,
  layer-overlay, file-helper, scene-load, and chrome-helper logic out of the
  main viewer file.

## 0.1.9 - 2026-07-08

- Added native Linux desktop support by building the real `occluview` egui/wgpu
  app on Linux, with XDG state/runtime paths, Unix socket open handoff, and
  stale-lock recovery after crashes.
- Added Debian packaging with freedesktop launcher, MIME registration,
  thumbnailer, AppStream metadata, app icon, maintainer hooks, and runtime
  dependencies for X11/Wayland/Vulkan desktops.
- Extended the release workflow so version tags build Windows MSI/portable ZIP
  and Linux `.deb` assets, then publish all artifacts and checksums to one
  GitHub Release.
- Included encrypted HPS support in shipped Windows and Linux packages.

## 0.1.7 - 2026-07-07

- Hardened the Windows thumbnail smoke so the MSI workflow now compares the
  direct `IThumbnailProvider` path against Explorer's `IShellItemImageFactory`
  path instead of accepting any non-null bitmap.
- Added real `stl`, `ply`, and HPS smoke fixtures, including the legacy package
  alias, so Explorer thumbnail validation covers the formats the app ships.
- Switched the cached Explorer thumbnail renderer to prefer a hardware adapter
  before falling back, reducing cold-start latency when the shell bursts
  through many thumbnails.

## 0.1.6 - 2026-07-07

- Fixed the Windows packaging path after the failed `0.1.5` packaging attempt:
  `occluview-shell` now requests the correct `windows-rs` input focus modules
  and passes a local `x86_64-pc-windows-msvc` shell check before tagging.

## 0.1.5 - 2026-07-07

- Added an Explorer Preview Pane handler for supported mesh and dental scan
  formats.
- Hardened the Windows shell integration smoke path with installed thumbnail
  and preview lifecycle validation, including MSI upgrade and uninstall checks.
- Tightened preview-handler COM lifecycle behavior around focus, reparenting,
  teardown, and unloadability.

## 0.1.0 - 2026-07-06

- Stabilized multi-file opens so new scans join the existing scene without
  re-homing the camera.
- Constrained in-viewport layer names so long filenames do not resize the
  overlay.
- Improved live viewport anti-aliasing and studio lighting readability.
- Switched shell thumbnails to orthographic framing and added file-path
  initialization for more reliable Explorer extension detection.
- Prepared the first public Windows package path.

## 0.0.1 - 2026-07-06

- Native Windows viewer with a full-window 3D viewport.
- Open paths for STL, PLY, OBJ, GLB, and HPS, including the legacy package
  alias.
- Explorer thumbnail provider and MSI packaging path.
- Neutral Windows file type names and one generic 3D file icon.
- HPS release build path with basic binary hardening.
