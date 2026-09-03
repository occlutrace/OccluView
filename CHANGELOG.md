# Changelog

This file records user-visible changes. Internal refactors and test-only work
remain in the Git history.

## 1.1.1 - Unreleased

### Viewer

- Added a compact Help reference for the complete keyboard and mouse controls,
  with a contextual reminder in the viewport.
- Replaced the circular viewport axis ring with a projected orientation cube;
  only labeled faces snap the camera, and the cube adapts to the viewport
  background.
- Prepared clearer operator-facing package names and release documentation for
  the 1.1.1 validation build. The release remains unpublished.
- The Layers panel now sizes itself to the scene, and the window grows with
  the panel: every layer stays visible without scrolling. Manually shrinking
  the window brings the scrollbar back.
- Fixed right-click behavior: a stationary right-click clears measurements and
  the Section-panel ruler reliably again, right-clicking the color swatch or
  the gaps in a layer row opens the layer menu, and saving remains reachable
  while the measure tool is armed.
- Added preferences: frame a scene when it opens, double-click refocus,
  orbit and zoom speed, recent-scene count, viewport background (gray, white,
  dark), cut-away ghost toggle, millimeter or inch readouts, UI scale, a dark
  theme, and remembered sculpt brush settings.
- Added an empty-viewport welcome screen with an Open call to action, drag-over
  feedback for dropped files, and a loading spinner in the status pill.
- Removed the unwanted viewport edge strip and replaced the framed status pill
  with a compact transparent status row.
- Refined the About actions, Settings sliders, Sculpt controls, and the bounded
  Mesh Repair report modal for a cleaner dental CAD workspace.
- Stabilized About, third-party notices, and Repair Mesh sizing by separating
  the modal backdrop from the measured card; added a headless Alignment/
  Heatmap, Mesh Editing, Sculpt, and Repair walkthrough to the README.
- Toolbar tools gained keyboard shortcuts (C, M, T, A, E) shown in their
  tooltips; icons now tint correctly with transparency.
- A broken settings file is preserved as `settings.json.bak` instead of being
  silently overwritten.

### Windows

- Restored the Explorer Preview Pane's synchronous first-frame path: render a
  bitmap, request paint, then report success. This avoids deferred work that
  can leave Prevhost showing a permanent loading indicator.
- Kept thumbnail and Preview Pane rendering on the current Rust and wgpu stack.
- Made Windows installer upgrades forward-only, refreshed Explorer associations,
  and preserved a working installation through rollback or failed major upgrades.
- Added a private `Prevhost.exe` smoke check that confirms visible pixels in
  the real surrogate while retaining low-integrity isolation.

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
- Refined Mesh Editing controls and the About dialog.

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

- First stable release of the viewer, Mesh Editing, HPS pipeline, Windows
  Explorer integration, Linux packages, CLI, and signed update channel.
- Shared thumbnail loading and rendering across Windows, Linux, and the CLI.
- Added minisign verification for Windows and Linux release artifacts.
