# Changelog

This file records user-visible changes. Internal refactors and test-only work
remain in the Git history.

## 1.1.1 - 2026-08-31

### Windows

- Fixed MSI rollback on workstations without a separately installed Microsoft
  Visual C++ runtime.
- Made same-version installs and failed major upgrades preserve a working
  existing installation.

## 1.1.0 - 2026-08-24

### Highlights

- Expanded Align Scans with robust point fitting, trimmed point-to-plane ICP,
  exclusion masks, signed deviation maps, and clearer fit diagnostics.
- Improved mesh editing across repeated sculpt strokes, topology changes,
  microscopic facets, and bridge splits.
- Unified application, Explorer thumbnail, and Preview Pane rendering, with
  bounded concurrency and high-DPI thumbnail output.

### Improvements

- Scene and batch exports preserve layer transforms, avoid name collisions,
  validate format compatibility before writing, and default to the source
  directory.
- Editing controls fit the viewport more consistently, and Shift applies the
  strongest Smooth stroke with a wider footprint.
- `.dcm` remains available through Open With without taking the system file
  association from medical DICOM software.

### Reliability and security

- Importers reject oversized, cyclic, deeply nested, or malformed input before
  it can exhaust memory or corrupt scene state.
- Renderer and shell failures are isolated to the active request; transient
  thumbnail failures no longer become permanently cached placeholders.
- Windows single-instance IPC is restricted to the current user.
- Release packages include license notices, SBOMs, checksums, minisign
  signatures, and GitHub build provenance.

## 1.0.6 - 2026-07-29

- Added Align Scans with paired landmarks, ICP refinement, manual positioning,
  exclusion painting, deviation heatmaps, undo, and background processing.
- Preserved aligned layer poses in scene and per-layer exports.
- Prevented stale alignment results and measurements from overwriting newer
  edits or surviving incompatible geometry changes.
- Improved Cut View zoom behavior and made the viewport scale bar follow the
  active camera.
- Fixed sculpt picking after topology-changing Smooth strokes.

## 1.0.5 - 2026-07-21

- Kept sculpting responsive and stable on large or locally damaged meshes.
- Synchronized Cut View lines, shaded slices, measurements, pan, and zoom with
  the main camera.
- Improved mixed-folder thumbnail scheduling and fallback isolation.
- Refined Mesh Editor controls and the About dialog.

## 1.0.4 - 2026-07-19

- Added interactive Add/Remove and Smooth sculpt brushes with undo and
  keyboard controls.
- Improved Bridge Split placement, startup latency, and anatomical orientation.
- Fixed large-hole caps and protected sculpting from inverted triangles.
- Added a per-layer neutral material toggle.
- Corrected affected HPS/DCM texture atlases without altering legitimate blue
  materials.

## 1.0.3 - 2026-07-15

- Export Layer now starts in the source directory, selects a compatible format,
  and warns when the chosen format cannot preserve the full payload.

## 1.0.2 - 2026-07-15

- Made Bridge Split tolerate stale normal data without weakening geometry
  validation.
- Limited interactive Close Holes to selected visible faces while preserving
  whole-mesh repair for explicit repair operations.

## 1.0.1 - 2026-07-15

- Fixed compressed HPS/DCM texture handling and deterministic color decoding.
- Added a bounded Bridge Split fallback for open scans and importer residue.

## 1.0.0 - 2026-07-15

- First stable release of the viewer, mesh editor, HPS pipeline, Windows
  Explorer integration, Linux packages, CLI, and signed update channel.
- Shared thumbnail loading and rendering across Windows, Linux, and the CLI.
- Added minisign verification for Windows and Linux release artifacts.
