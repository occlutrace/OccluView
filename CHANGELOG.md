# Changelog

This file records user-visible changes. Internal refactors and test-only work
remain in the Git history.

## Unreleased

- The first mesh in a scene gets a camera even when automatic framing is disabled; existing views stay unchanged.

- Empty scene panes respond to right-clicks before a viewport image is rendered.

- Graphics startup retains software fallback when every hardware adapter fails to create a device.

- Background and cut-ghost preference changes refresh every scene.

- Bridge Split remembers size-slider changes before placement and displays the selected disc size.

- Orthographic scene picking selects visible surfaces even when they lie behind the orbit camera.

- Mesh region selection preserves existing marks when its outline or projection input is invalid.

- Sculpt strength falls back to the brush default when an invalid numeric value is supplied.

- Switching Sculpt tips or redrawing the panel preserves the chosen brush size until the size control is changed.

- GPU vertex updates reject changed vertex counts before uploading data.

- Close Holes stays disabled until faces are selected, matching the repair operation and its tooltip.

- Edit is disabled when no visible triangle mesh can be opened.

- Mesh selection shortcuts keep face marks unchanged while a mesh operation is running.

- Mesh editing preserves the current undo and redo history when an outdated operation finishes.

- Invalid contact-field updates clear previous contact paint instead of displaying a stale measurement.

- Cut View and Bridge Split preserve the disc and section framing when wheel input is non-finite.

- Closing the viewer preserves a sculpt brush change made on the closing frame.

- Restarting Bridge Split can prepare a new source without waiting for an abandoned preparation.

- Sculpt brush preferences save after the last slider change settles.

- Cut View zoom keeps the section point under the cursor fixed while resizing the disc.

- Scale bars retain nonzero millimeter and inch labels when zoomed in.

- Closing the Bridge Split Section window cancels the separator and preserves the scene.

- Bridge Split discards previews after position edits, including sculpt changes that preserve mesh topology.

- Tool keyboard shortcuts obey the same availability rules as their toolbar buttons.

- Viewport right-click menus remain visible and respond to actions while mouse navigation stays confined to its scene.

- Mesh edits preserve correct facet normals for very small and very large geometry.

- Section contours remain visible on layers with small nonzero scales.

- Closest-surface queries preserve valid face and edge projections on small triangles.

- Closing holes no longer overflows rim-weld grid coordinates on distant geometry.

- Repair no longer merges distinct vertices when a mesh is stored far from the coordinate origin.

- Closing holes with an empty face selection now preserves the complete mesh.

- Section contours skip triangles containing invalid coordinates instead of emitting NaN points.

- Repeated faces no longer turn open section contours into artificial closed loops.

- Surface queries handle small triangles separated by very large distances without overflowing their grid.

- Resizing a short scene pane with the Section panel open no longer crashes the axis gizmo.

- Malformed triangles are skipped safely when calculating surface normals.

- Bridge Split rejects zero-area triangles before accepting an input or output as a closed mesh.

- Knife strokes keep their radial fallback when a bearing cannot be normalized.

### Mesh editing

- Selection actions ignore stale layer-menu requests before changing any marked layer.
- Undo and redo keep every restored mesh, including cut parts, in the next save operation.
- Layer-menu mesh edits wait for pending Sculpt work to settle.
- Selection actions preserve hidden layers and reject marks from replaced mesh geometry.

### Formats

- Opening or dropping a 3MF (or any ZIP) file now reports that the format is
  not read and suggests exporting to STL, PLY, OBJ, or GLB. The dialog
  previously showed an internal error that named an implementation crate.

### Layers

- Dragging a layer name moves the layer instead of selecting its text. The name
  shows a grab cursor, the drag carries a translucent chip with the layer's name
  and tint, the drop target fades in and out instead of flashing, and the edge
  band highlights only the strip that will actually take the layer.

### Sculpt

- The brush cursor is a pale wash of the tool colour, matching the brightness of
  the reference viewer, so the footprint marks the surface instead of covering
  it. The screen ring is a hairline with a crosshair centre, and the translucent
  tool body uses its own rim shading and is occluded by geometry in front of it.
- The pointer becomes a crosshair while a Sculpt tool is armed.
- The layer guard runs more rollback waves before it restores a whole dab, so a
  Smooth stroke leaves fewer untouched patches on dense geometry.
- Remove no longer builds an opening distance field inside a stroke, and a group
  whose opening thickness was never measured keeps the widest reserve instead of
  a made-up thin one.
- Incremental shading no longer publishes a zero normal for a vertex whose faces
  are all filtered out, which read as a spike on the surface.
- Relax is warmed with the other brushes, so its first stroke is as responsive.

### Align

- Suggested deviation scales expand beyond 10 mm when the measured range requires it.
- Non-finite heatmap limits recover to a finite working range instead of interrupting alignment or panel rendering.
- Signed deviation bands use matching distance intervals above and below zero.
- Perform alignment explains its two-arrow requirement when only one complete arrow is placed.
- A quick exclusion-brush tap paints the mesh even when press and release arrive in the same frame, without placing an alignment point.
- Disabled Heatmap controls explain whether refinement is missing or deviation measurement is running.
- Cancel keeps alignment open for retry when another layer edit temporarily prevents restoring scan positions.
- Invalid or overflowing manual drag input no longer writes a non-finite scan position.
- Deviation legend labels retain 0.001 mm precision instead of rounding small limits to zero.
- Typing a matching percentage now sets the intended ratio, including values with a percent sign or decimal comma.
- Painting exclusion regions immediately cancels fits computed with the old markings.
- Changing or removing alignment points cancels fits that used the previous points.
- Deviation summaries average both middle readings when computing an even-count median.
- Deviation maps and statistics exclude distances that overflow storage and non-finite readings.
- Closing the exclusion brush without changing markings restores the deviation map.
- Cancel preserves the save prompt for restored scan positions, including an unfinished drag or a position already exported during the session.
- Unavailable alignment controls now show their refusal tooltips and expose the correct disabled state.
- Deviation sensitivity uses only measured overlap, excluding vertices beyond the fixed scan's border.
- Best-fit matching scores its coarse hypotheses on the seed sample set instead
  of the dense one, which removes most of the work one press performed. Searching
  a small scan against a large one no longer takes minutes.
- Best-fit matching compares what two candidate poses explain about the other
  scan again, so two placements that explain it equally are still reported as
  ambiguous while a seating with more support wins outright.
- The fixed-to-moving overlap signal no longer discards hits on an open border,
  which is what a small scan's own rim produces.
- The deviation map opens on the clinical range, 0.000 to 0.100 mm, instead of
  0.050 to 0.200 mm. The hottest colour is now 100 um rather than 200 um, and the
  bar starts at zero instead of hiding everything below 50 um.

- Best-fit ambiguity checks remain consistent when a scan is stored far from the coordinate origin.
- Point-pair alignment rejects bounds whose overlap calculations overflow instead of accepting an unchecked pose.
- Cancelled surface refinement cannot report a trustworthy pose after the dense solve.
- Alignment vertex lookup rejects overflowing indices without panicking.
- Exclusion brushes with very large finite coordinates keep vertices outside their radius unchanged.

### Viewer

- The section view recomputes its contour after a sculpt. The section cache was
  keyed on the mesh's topology revision, which a sculpt commit deliberately keeps
  frozen, so the view kept drawing the pre-sculpt contour for the rest of the
  session.
- The Explorer preview menu no longer offers "Edit in OccluView". The item
  launched the viewer with no editing verb, so it did exactly what Open does;
  the menu promised an action the application does not have.

### Reliability

- The viewer stops polling for hand-off requests once no listener is left. The
  fallback listener only noticed a closed channel when it had something to send,
  so an idle listener kept a 50 ms directory poll and its repaint bursts running
  for the life of the process on Linux and macOS.

### Localisation

- The Repair report's "Copy details" payload and the thickness probe's "open: no
  opposite wall" label now follow the interface language. Both were hardcoded
  English, so they stayed English in all seven languages.
- Measurements, the deviation and contact legends, and grouped counts use the
  interface language's decimal and grouping separators, so German and Russian
  read "0,05 mm" instead of "0.05 mm".


## 1.2.1 - 2026-09-30

### Workspace, Sculpt and Align

- Work with two independent scenes side by side. Move layers between scenes,
  import into a chosen scene, and undo transfers without mixing scene history.

- Sculpt Add and Remove follow the local surface instead of camera depth.
  Brush size uses millimetres and keeps its relative size when switching tips.
- Every Sculpt brush applies its dose by elapsed brush time, including moving
  strokes. Smooth preserves rounded forms while reducing smaller bumps.
- Sculpt strength and size wheel controls use proportional steps, and brush
  preferences survive restarting the viewer. The surface footprint stays
  visible on light models.
- Sculpt commits preserve live surface normals across Undo and Redo.
- Sculpt uses one surface path for clicks, travel and held strokes. Quick
  clicks survive preparation, and crossing a panel pauses the brush path.
- Ctrl-drag in manual alignment turns around the surface point you grabbed.
  Point pairs remain available after manual movement and tab changes, and the
  fit button recovers after worker failure.
- Best fit accepts a small fragment against a full scan with the default
  matching ratio, whether the fragment is the moving or fixed scan.
- Best fit matching supports pre- and post-treatment scans when the unchanged
  region provides sufficient evidence for the alignment.
- Best fit matching uses principal surface orientations to recover scans
  rotated around tilted axes.

### Accessibility

- Screen readers receive localized names, roles, selected states, and disabled
  states for the viewer controls, including the layer tint palette.

### Viewer

- Imports estimate scene, picking-tree, and renderer memory against the viewer's
  2 GiB budget. Size messages identify binary amounts as GiB.
- Best fit refuses equally plausible surface matches and tells you to mark the
  matching area or move the scans closer before trying again.
- Point-pair placement remains provisional until surface refinement passes;
  deviation maps require a verified pose.
- Sculpt remeshing updates the live surface locally during a stroke. The brush
  stays visible while hovering, and Ball, Knife and Cylinder use matching tip
  glyphs and cursor shapes.
- Sculpt keeps sharp crease shading while welding the surface for continuous
  brush strokes.
- Default-size Smooth strokes remain responsive on full-arch scans.

- Best fit matching accepts a scan of part of a jaw against a scan of the whole
  jaw again. Two different jaws are still refused.
- The Brush commands ("Fit everywhere", "Fit nowhere", "Invert markings",
  "Mark automatic") apply to both scans of the pair. Choosing one scan in Mesh
  selection narrows a command to it, and the status line names that scan.
- A brush stroke paints the scan under the cursor, on either arch.
- Marked surface is drawn blue over the scan's own colour, texture and lighting.
- "Fit everywhere" on a scan with nothing marked leaves the scan unchanged.
- Ruler: after the first point, a click on a drawn ruler line ends the
  measurement on that line, for example the Korkhaus anterior arch length from
  the incisal point to the line through Pont's premolar points, which runs above
  the palate. The end goes where the click is along the line; the ruler shows
  its 3D length and the smaller angle to the line, and the end can be dragged
  along the line. A strip over the viewport and Settings choose between any
  angle and 90° (the foot of the perpendicular); Shift uses the other choice
  while held.
- Ruler: pressing an end without moving the mouse leaves it in place. It
  follows the pointer once the pointer moves.
- The contact reading names the scan it is measured against when the scene has
  more than one candidate, and offers the others.
- A contact measurement that fails says so and offers "Read again".
- Settings shows "Keyboard and mouse" and "About OccluView" as one row of two
  buttons.

### Files

- Saving keeps the format a scan was opened in when OccluView can write it. A
  scan from a format it cannot write (HPS, GLB, OFF) is saved as PLY when it
  holds colour, a texture or a mapping, and as STL when it is geometry alone.
  "Save scene as" follows the same rule. The save format is no longer a setting.
- PLY and OBJ export of a textured scan write the texture colour per vertex
  (RGBA in PLY, RGB in OBJ), so the colour opens in other tools and the files
  stay close to the size of their geometry. PLY files with an embedded texture
  written by earlier releases still open in colour. When a layer's own vertex
  colours are replaced by the texture, the export says so.
- OBJ files with an `mtllib`/`map_Kd` texture, or with an image of the same name
  beside them, and PLY files with a `comment TextureFile` line open with their
  texture. Files larger than 1 GB and companion images larger than 64 MB are
  refused.
- HPS/DCM scans from 3Shape lab scanners with red and blue swapped in the
  texture open in their correct colours.
- A malformed binary PLY is refused with an error instead of hanging the viewer
  or the Explorer preview. A crafted PLY header can no longer make the importer
  hold a second copy of a payload it rejects.
- Opening several scans at once parses two at a time and keeps at most about
  half a gigabyte of file data in memory. A file larger than 1 GB is refused
  with a message that gives both sizes.

### Windows

- Explorer preview menu labels follow the Windows UI language in English,
  German, Spanish, French, Italian, Brazilian Portuguese, and Russian.

### macOS

- Settings choose whether trackpad scrolling pans or zooms the viewport;
  keyboard and mouse help follows that choice.

- OccluView runs natively on Apple Silicon Macs with macOS 14 or later, as a
  `.dmg` with the app and a `.pkg` installer. Downloads are offered once the
  packages are signed and notarized; the in-app updater installs the `.pkg`
  through the macOS Installer.
- Finder opens STL, PLY, OBJ, GLB, HPS and `.dcm` files with OccluView. `.dcm`
  is offered under "Open With" only, so medical DICOM files keep their own
  default application, and a DICOM file is refused.
- Shortcuts show the Command key, a trackpad scroll pans the view, and pinch
  zooms.

## 1.2.0 - 2026-09-14

### Viewer

- Opening or closing a case while a Sculpt stroke was still held used to destroy
  the layer being sculpted without asking. A live stroke is now treated as work
  in progress by the open, close, and save guards, so the operator is asked
  before the scene it is changing goes away.
- Abandoning a Smooth stroke that had already densified the surface left the
  denser mesh in the case. The stroke recorded no history step while it was
  open, so Ctrl+Z could not name that geometry and the save prompt did not count
  it. Aborting or failing a stroke now returns the layer to the geometry the
  stroke started from, keeping whatever earlier finished strokes had produced.
- A contact reading could show the numbers of the reading it replaced, and a
  scan nudged and put back could leave the reading waiting on a measurement it
  had already discarded. A finished reading now has to name the measurement it
  answers and is applied only against those surfaces as they stand now.
- Align Scans presents rough point alignment before nearby surface refinement. Refinement now stays within the selected correspondence radius instead of launching a global feature search from an already placed scan.
- The heatmap opens at 0.00–0.10 mm. Its cool and hot limits are editable above and below the colour legend.
- In Mesh Editing, keys 1 and 2 open the Sculpt tab with Add/Remove and Smooth respectively. The sculpt cursor appears as soon as the background picking tree is ready, before brush preparation finishes.
- Modal text and Mesh Editing headings use readable ink in the light theme.
- A scan moved by hand in Align Scans and put back before the mouse is released
  is no longer reported as unsaved work, and does not leave a step behind. Work
  that was already unsaved stays unsaved, and a drag that ended somewhere else
  is still one undo step.
- A layer cut out of another keeps the file it descends from when its source
  layer is no longer in the scene. The export dialog opens in that file's
  folder, under its name and format, instead of a neighbouring case's.

## 1.1.1 - 2026-09-03

### Viewer

- Best fit matching seats two scans again. A refusal that used to fire on a
  successful fit — "could not confirm an improvement" — is gone: the solver
  decided whether to keep a pose by the size of the residual it had reached,
  and two real scans never reach a nanometre, so a pair it had already seated
  came back as a failure with nothing moved and no map. It now returns the best
  pose it measured, and the search radius starts at the number the operator
  set instead of a quarter of it.
- Two different jaws are still refused, and for the reason that distinguishes
  them from an alignment: they have no single correct joint position, they only
  meet where their occlusal surfaces touch. That case reaches a fifth of one
  surface on the other, so it satisfied every coarse measure the tool had; the
  median distance of a matched point is what separates it from a real seating.
- A contact mark takes the light the scan around it takes. The ramp used to be
  mixed over the finished surface, which no mark can take a highlight from, so
  it read as a flat sticker and arrived darker than the legend it is read
  against — the blue stop's 216 was reaching the screen as 173. It is painted
  into the base colour now and the diffuse light is divided back out, so the
  law's colour arrives at the law's value and the mark carries a highlight.

- Align heatmaps now appear only after a current confirmed match, with a compact
  absolute millimetre legend and saturated display colours. A sculpt stroke,
  a hand drag, a role swap made by the first matching click, or a change to the
  matching inputs withdraws the map and the match it measured; the legend stays
  readable at the 0.00 mm end of the range control, including its saturated end.
  A measurement the sampled surface cannot support at all — too little overlap,
  or samples that do not span every direction — is refused instead of drawn;
  a surface that is merely weak in one direction is still measured, because that
  is what the deviation map is for.
- Sculpt worker topology changes, cancellation, and repeated strokes preserve
  ordered geometry and undo boundaries. A stroke that cannot finish now reports
  the reason in a dialog and stands the brush down instead of leaving it armed
  over a revoked worker.
- Sculpt brush sliders fill the panel again: the rail itself uses the full
  width, not just the row that contains it.
- The renderer's stencil cap passes keep their geometry-precise depth again,
  which is what makes a filled cross-section possible. The viewer itself still
  draws the hollow preview (`show_hollow`), so the cap is exercised by the
  renderer's golden-image tests rather than by the main window.

### Reliability

- Weak Linux graphics adapters receive adapter-aware wgpu limits and a
  single-sample compatibility profile; startup failures now produce a visible
  diagnostic signal and a non-zero exit status. `OCCLUVIEW_LIVE_MSAA=1` starts
  without multisampling when a driver rejects it, and a fatal startup is
  offered to the desktop through the notification service the system provides.
  Adapter selection and the fallback path are deterministic across launches.
- Added `occluview --diagnostics` and installed-package graphics smoke checks;
  the report now names the chosen live sample count and each adapter's
  multisample support.
- After a graphics fault the viewer stops submitting frames and reports the
  fault once instead of retrying a broken device in a loop.
- Exporting over a file that is a symbolic link updates the file it points at
  instead of replacing the link, and "Export each layer" keeps working in
  folders on removable media or network shares that cannot hard-link, which
  also restores its collision retry on Windows.
- Added an occlusal contact reading. Right-click a scan and choose Show
  contacts: the scan is measured against the scan it bites against — the
  nearest visible surface — and BOTH arches are painted where they meet. The
  panel offers two readings of the same bite and one slider. Contacts marks
  only where the surfaces actually meet and colours each mark by how deep the
  bite is there, the way articulating paper leaves the rest of the tooth bare;
  Approach paints how close the other scan is everywhere, load included, for
  judging a jaw relationship rather than the contacts themselves. Heavy at
  moves the depth the ramp calls fully loaded, and it recolours the
  measurement already in hand instead of re-measuring: the field is what the
  surfaces do, and the ramp is only what the colours say about it. One colour
  per contact flattens every patch to its deepest point for a case whose
  marks are better read as areas than as distributions.
- The contact map carries a pointer readout: point at the surface and the
  value under the cursor is shown in micrometres below a millimetre, beside a
  swatch of the exact colour the surface wears there. A vertex with no
  opposing surface inside the search radius reports nothing at all rather than
  a plausible zero.
- Three properties of such a map decide whether it can be read, and each is
  enforced rather than assumed. Red sits on the load side, because red at the
  far end puts a ring around every mark — a tooth curves away from a contact
  within half a millimetre, so the geometry guarantees the ring. Almost
  nothing is painted, because painting the whole approach turns a case with a
  handful of real contacts into a field of colour with the marks lost inside
  it. And the paint ends by weight rather than by fading toward white, which
  reads as a lighting artefact instead of as data. Colour mixing runs in
  Oklab, so the ramp has no neon band or hue overshoot between its stops.
- The measurement is a Rust kernel (`occluview-contact`) that runs on its own
  background thread beside the alignment worker, so a million-vertex pair
  never freezes the window. It is deliberately independent of the align
  session: a reading runs over its own pair, takes its roles as arguments, and
  does not inherit the exclusion brush — those marks are indexed by the align
  session's roles, and applying them to a different pair would paint out an
  arbitrary region of a scan with nothing on screen to say why.
- The painted band reaches the screen through a stop table in the per-mesh
  uniform, evaluated in the fragment shader, so moving the slider is a uniform
  write and never a re-upload or a bind-group rebuild.

- Added a compact Help reference for the complete keyboard and mouse controls,
  with a contextual reminder in the viewport.
- Replaced the circular viewport axis ring with a compact rotating axis triad in
  the bottom-right corner; endpoint clicks snap the camera without stealing
  scene input.
- Clarified the Windows package names and operator-facing release documentation
  for 1.1.1.
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
- Reworked Settings into a more compact layout. Language selection now expands
  in place, puts System language first, and clearly reports catalog fallback.
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
