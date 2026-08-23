# Changelog

## 1.0.9 - 2026-08-23

- Made Shift actually strengthen the Smooth brush. Shift doubled the force
  slider and capped it at 100% — but Smooth turns force into relaxation
  passes that converge, so at the default slider the difference was barely
  visible and at full slider Shift changed nothing at all. Shift now forces
  maximum strength and widens the brush footprint by 1.75x, which is the
  lever that genuinely smooths harder: the pinned boundary moves outward
  and one held stroke irons a visibly wider patch. The cursor ring shows
  the widened footprint while Shift is held.

- Gave the sculpt size and force sliders the Mesh Editor window's full
  width. The label moved onto its own line with a live value readout on the
  right; on the narrowest window the rail gained a third more travel.

- Save dialogs now start in the folder the scan came from. A layer with no
  file of its own — a part split or cut out of another scan — starts in its
  source scan's folder instead of whichever case happened to load first,
  then falls back to wherever the last export landed. A folder that no
  longer exists (unplugged media) is skipped instead of being handed to the
  dialog.

- Every package now ships its legal texts. The MSI installed no license
  files at all; the deb had nothing for the statically linked crates. All
  artifacts now carry LICENSE, NOTICE, and a generated
  THIRD-PARTY-NOTICES.md with the copyright notices and license texts of
  every linked crate and bundled font — regenerated from the lockfile and
  gated in CI so it cannot rot like the old hand-written NOTICE did (it
  credited libraries this program never shipped). The About dialog gained a
  Third-party licenses view showing the same file.

- Both binaries answer `--version`; the CLI used to reject the flag as an
  unknown subcommand and the viewer would have opened it as a file. The
  Explorer shell DLL and the CLI tools now carry Windows version resources,
  so their Properties pages finally name the product and version.

- Neither installer claims the `.dcm` association any more. `.dcm` is the
  extension 3Shape writes its HPS containers under and the one medical DICOM
  has used for decades, and OccluView rejects DICOM by design — so on a
  workstation that holds both, installing OccluView used to give every CBCT
  file an OccluView icon, a preview handler that failed, and a double-click
  that opened an error instead of the study. On Windows the MSI, the manual
  `.reg` and `regsvr32` no longer write the machine-wide `.dcm` ProgID, its
  icon, or the thumbnail and preview handlers under either the bare key or
  `SystemFileAssociations`; on Linux the `.dcm` glob dropped below
  `application/dicom` in the shared MIME database, which it had been winning.
  OccluView is still offered for `.dcm` in *Open with*, in Default Apps and on
  the right-click menu, and a user who picks it gets the icon, thumbnail and
  preview as before. Uninstalling a build that did claim `.dcm` still cleans
  its entries up.

- Dependency license and advisory scanning now covers the Linux target,
  which previously sat outside every check. It immediately surfaced two
  real advisories, both fixed by upgrades rather than exceptions: quick-xml
  0.41 (RUSTSEC-2026-0194/0195) and webbrowser 1.2.4 (RUSTSEC-2026-0257).

## 1.0.8 - 2026-08-22

- Fixed Align Scans refusing correct fits between two scans stored in
  different coordinate systems, such as a DICOM-derived surface against an
  STL of the same case. The guard read the pose's translation as "how far the
  scan moves" — a number that grows with how far a file's zero sits from its
  own geometry — so a scan turned over where it stands reported a 142 mm
  "move" against an 88 mm limit, having travelled 2.3 mm. The guard now asks
  the only question a registration has to answer: whether the fit leaves the
  two scans on top of each other. That reads the same whichever scan moves
  and wherever either file puts its zero, so the two directions now agree.

- Fixed 3D models rendering distorted on 4K displays, seen in fullscreen,
  where the width first passes the render target's 2560 px cap. The target
  was clamped one axis at a time, which changed its shape: 3840 x 2160 became
  a 2560 x 2160 texture painted across a 16:9 viewport, everything half again
  too wide. Both axes now share one scale factor, so the target always keeps
  the viewport's shape.

- Added a second tint group for telling two scans apart where they overlap.
  The existing shades are neighbours on one warm band by design, so two scans
  wearing any of them are hardest to separate exactly where an alignment needs
  them separated. The palette now lists Model shades and eight Overlay colours
  under their own headings, led by Cobalt against Tangerine because blue
  against orange is the strong opposition that survives red-green colour
  blindness. On a scan that carries its own colours an overlay colour
  overrides them, so it reads as the colour chosen rather than that scan
  darkened — a plain scan needs no override at all — and cycling the tint walks
  the whole palette instead of dropping back to Stone IV on reaching a colour
  it did not know about.

## 1.0.7 - 2026-08-22

- Fixed shading on sub-20um facets: the absolute epsilon test culled every
  small triangle from lab scanners (7 um spacing), falling back to a flat
  +Z normal and a uniform specular wash. Replaced with a scale-invariant
  `longest_edge^2 * 1e-10` test in core, formats, and HPS.
- Fixed Mesh Editor window: now scales with the viewport (22% clamped
  200-320 px), opens bottom-left, and stays draggable within the viewport.
  Modal dialogs no longer anchor-center over the 3D scene.
- Hardened release pipeline: least-privilege `contents: read` on build jobs,
  `write` only on publish, `persist-credentials: false`, HPS key step-scoped,
  WiX 3.14.1 pinned, all Actions SHA-pinned, toolchain `1.86.0` explicit,
  Authenticode required on tags with post-build signature verification.
- Added SBOM (CycloneDX) and SLSA build provenance attestation for release
  artifacts.
- Added fuzz targets for the hostile-input surface (dispatch, HPS, STL, PLY):
  60s smoke on every PR and 300s nightly deep fuzz with artifact upload.
- Hardened Windows single-instance IPC with per-user mutex/pipe names (SID
  hash) against Low-IL squatting.
- Added `docs/ARCHITECTURE.md`, ADR for texture channel correction, README
  badges and precise DCM disclaimer, expanded SECURITY disclosure, CODEOWNERS
  and issue/PR templates, compressed `animation.webm` alongside GIF.

- Fixed Explorer folders full of scans losing their thumbnails for good. When
  several files — especially several formats — were extracted at once, any
  request that ran out of time answered Windows with a placeholder image, and
  Windows cached that image as the file's thumbnail until the file itself was
  modified. A busy moment became a folder of permanent grey cubes. The
  extension now answers "not yet" only as a retryable failure and reserves the
  placeholder for files that are genuinely broken or unsupported, so a folder
  that stumbled once heals on the next browse instead of staying blank.
- Explorer thumbnails and the preview pane start faster: the renderer is
  created once per host process — warmed in the background the moment Windows
  loads the extension — instead of being rebuilt for every file. Clicking
  through scans in the preview pane no longer pays a fresh GPU setup per
  click, and a preview-pane resize renders once instead of twice.
- Files stored by cloud sync as size-less placeholders no longer come back as
  the oversize placeholder cube: a stream that does not declare its size is
  read to the normal limit instead of being rejected unread.
- One bad file can no longer take down every thumbnail and preview around it:
  every entry point into the extension now contains failures to that one
  request instead of letting them crash the shared Windows host process.
- Thumbnails render at full sharpness up to 2048 px, covering the largest
  Explorer icon sizes on high-DPI displays.

## 1.0.6 - 2026-07-29

- Sculpt: the brush survives its first densifying Smooth stroke. The rebuilt,
  denser mesh arrived without its picking index and nothing ever rebuilt one,
  so from that stroke on the cursor never found the surface again and no
  sculpting applied at all.

- Added Align Scans: click a point on one scan and the matching point on
  another, and they pair themselves. There is no target picker and no roles —
  the first clicked point names the scan that moves, the next click on a
  different scan names the one that stays, and with exactly two scans in view
  the pair is implied and needs no click at all. Align fits the clicked pairs;
  Refine seats the surfaces against each other.
- Added a deviation heatmap with an honest account of itself. It reads how far
  apart two scans are — with or without aligning them, since naming the two
  surfaces is all a measurement needs. One scan carries the colour and the
  other fades, so the map is a single clean surface instead of two solids
  fighting through each other. Vertices with no facing surface in reach are
  grey and excluded from the statistics rather than painted at full scale, and
  the window reports how many there were. The map draws unlit, so the ramp
  reaches the screen at the colour it was measured at.
- The Align Scans window moves like the mesh editor, carries icons on its
  actions, and ends in Cancel and Done: closing it puts every scan back where
  it was, and only Done keeps an alignment. Refine is the primary action and
  measures on completion; the point fit does not, because the very next step
  invalidates whatever it would have drawn.
- Added an exclusion brush: paint an artefact or a bite block out and it leaves
  both the fit and the map.
- A scan can also be moved by hand, with the drag free, locked to Z, or locked
  to the XY plane. Whatever scan you grab is the one that moves.
- Every alignment step is one Ctrl+Z away, and fitting, refining, and measuring
  run off the UI thread so a full arch never freezes the window.
- Fixed layer export ignoring the layer's placement: a scan moved in the
  viewport was written back in its original orientation, silently discarding
  the alignment. The pose is now baked into the exported geometry.
- A right-click on empty space now opens a scene menu — save the whole scene as
  one file, save each visible layer in its own pose, reset positions, fit the
  view. The viewer has no project file, so saving is how an alignment survives
  the session.
- Align Scans decides direction from your first click. With two scans in view
  the pair is still guessed before you touch anything, but that guess only goes
  by the order the files were opened in, so the first point you place now
  overrides it. The window names both scans in the direction the fit will run
  and offers one button to turn it round, arrows and markings included.
- Clear drops the pair so a third file can be aligned without closing the tool.
  The refusal that told you to "press Clear" used to name a control that did not
  exist.
- The Manually tab moves the scan you aim at. Two scans in an alignment overlap,
  so the nearest surface under the cursor was often the other one; the scan
  being placed now gets first refusal on the grab. The status line names the
  scan and the distance when a drag starts and again when it ends, and the axis
  constraint applies to Ctrl+drag rotation, which used to ignore it.
- Switching to Manually clears the placed arrows: a hand nudge moves the scan
  out from under them.
- An automatic fit is unsaved work. It never raised the flag the close guard
  reads, so an alignment could be lost on close with no prompt. Save scene and
  Save each layer no longer clear that flag for a hidden layer they skipped.
- A result the operator has overtaken is never applied. A refine carries a pose
  and commits it, so one landing after a hand move or a Ctrl+Z used to put the
  scan back where it had just been taken from.
- The heat map opens at a tenth of a millimetre and stays there. A finished fit
  used to switch the range back to automatic, so a range you chose could not be
  held.
- A measurement that reached nothing is no longer painted. Every unmeasured
  vertex is grey, so a scan moved out of reach came back flat grey; the panel
  now says what the reach was and how many vertices found the other scan.
- Grey is named by cause: no surface opposite, marked out, or unusable data. The
  legend carries the swatch. A bridge that exists on one arch only has nothing
  to measure against, and that is not an error.
- A structural edit takes the heat map with it. Repair, close holes, crop, cut,
  separate and a bridge split left a map of the surface the layer used to have.
- Markings are tied to the mesh, not to its vertex count: a repair can hand back
  different geometry with the same count, and the mask used to pass and exclude
  a region nobody had painted.
- A fit or a measurement against a hidden scan is refused by name instead of
  reporting a percentage for a surface nobody can see, and "% of the surface"
  now says "of the unmarked surface" when a region is painted out.
- Cut View: the wheel in the section window resizes the cutting disc with it —
  magnifying the section narrows the disc onto the detail being examined,
  pulling back opens it out. Resizing the disc no longer throws away the pan
  and zoom you set there.
- The viewport scale bar follows the camera. It was derived from the scene's
  bounding box, so it described the framing the file opened at and was wrong
  from the first scroll onwards.
- Removed the two-way surface-agreement statistic: it was never shown anywhere.

## 1.0.5 - 2026-07-21

- Made repeated sculpt strokes continue to respond on meshes with small or
  damaged facets. Invalid local faces are isolated instead of discarding a
  healthy brush region, while the mesh remains protected from inverted faces.
- Moved sculpt preparation, stroke execution, sparse GPU updates, and undo
  snapshots off the UI thread so large scans stay responsive across strokes.
- Kept mixed-folder thumbnail work bounded and isolated across unrelated file
  types, with the existing deterministic fallback for a single failed item.
- Reworked the mesh editor controls so Size and Force use the full available
  rail, and made About a compact centered dialog with balanced links.
- Synchronized the existing Cut View section panel with the main camera for
  lines, shaded slices, measurements, pan, and zoom; removed the redundant
  orientation gizmo.
- Kept Linux single-instance activation and the existing Windows/Linux package
  release checks in the same tag-driven release path.

## 1.0.4 - 2026-07-19

- Sculpt brushes are now robust to abuse: Smooth still flattens hard, while the
  clay Add/Remove auto-smooth is volume-preserving (Taubin), so building no
  longer leaves grain and no longer collapses the dome. A post-dab guard
  guarantees no triangle is ever left inverted, and the anti-inversion budget
  tracks the moved geometry instead of the original mesh.
- Bridge Split now sizes the starting cutting disc to the object instead of a
  fixed small default that always had to be enlarged, and warms the picking
  index off-thread so the first disc placement no longer freezes on a large
  scan.
- Fixed Thickness mode turning its visualization off on small orbit gestures
  while its button stayed lit; a right-click only clears when it is not an orbit.
- Redesigned the About window as a centered, minimal card with a GitHub link,
  and removed the version watermark from the 3D viewport.
- The 1/2 sculpt hotkeys only arm their tool now (pressing again keeps it on).
- Fixed Close Holes leaving sharp spike artifacts where a large interpolated
  cap met the surrounding surface after a lasso cut, and raised the cap size
  this covers (#9).
- Added a per-layer toggle to hide scan colors/texture and show a flat
  neutral material instead (#10).
- Added interactive sculpting to the Mesh Editor (#11), on its own Sculpt tab
  beside Edit Mesh: two tools dragged directly on the scan — an Add/Remove clay
  knife (hold Shift to carve) and a Smooth relaxer (hold Shift to force it) —
  with Size and Force sliders, Shift/Ctrl + mouse wheel to resize/re-intensify,
  a soft glow cursor that brightens with intensity, and one undo step per
  stroke. Add/Remove moves the brushed region coherently toward the camera
  (robust to a scan's inverted-normal patches) and cleans the surface as it
  goes, so it builds and carves smoothly instead of leaving potholes, spikes,
  or grain; Smooth flattens strongly while pinning the scan's open edges. Press
  1 (Add/Remove) or 2 (Smooth) to switch tools. Runs at interactive speed even
  on million-triangle scans (parallel, spatially indexed, CSR connectivity).
  Replaces the earlier one-click Smooth-selection button.
- Fixed DCM/HPS scans whose embedded JPEG texture atlas had its chroma
  swapped at the source, decoding blue where gingiva/enamel should read warm
  — and tightened the correction so it only fires on a whole-texture bias,
  not a real localized blue material (anti-glare spray, bite-registration
  silicone) sitting next to normal tissue color (#12).
- Fixed the Bridge Split / Cut View cutting disc misreading the arch as the
  cursor moved: its orientation is now purely anatomical — the disc stands
  upright (along the scan's occlusal axis) and cuts transverse across the
  arch at the exact spot under the cursor, turning continuously as the
  cursor sweeps around the curve. The camera no longer participates at all,
  so the disc keeps the same correct cut from any viewing angle instead of
  tipping flat at the sides of the arch from a facial or tilted view.

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
