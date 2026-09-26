# Changelog

This file records user-visible changes. Internal refactors and test-only work
remain in the Git history.

## 1.2.1 - 2026-09-21

### Viewer

- "Best fit matching" accepts a scan of part of a jaw against a scan of all of
  it again. The acceptance gate had briefly required a fixed fraction of the
  moving surface to seat inside 0.05 mm, which a real partial overlap cannot
  reach — the overlap seated exactly (median 0.000 mm) and the gate still
  refused it, so the button did nothing and no heatmap appeared. The median
  residual decides acceptance, as it did in v1.2.0, with the worst-fifth tail
  still bounded so a fit that slid onto a neighbouring surface is refused. Two
  different jaws stay refused.
- Excluding surface from best-fit matching with the Brush tool marks and
  commands both scans again. Selecting one mesh had narrowed "Fit everywhere",
  "Fit nowhere", "Invert markings" and "Mark automatic" to that single scan, so
  with an upper and a lower arch open the button an operator pressed did not do
  what its own sentence said: one arch was taken out of the match and the other
  was left in it. The Brush window's Mesh selection now offers both scans, which
  is what it opens on, and naming one scan still narrows a command for the
  overlapping case.
- A stroke paints the scan under the cursor on either arch of the pair. The
  previous behaviour aimed every dab at whichever mesh the Mesh selection named,
  so painting the other arch did nothing at all until the operator noticed the
  selection and changed it.
- The marked region is drawn as paint over the scan's own surface, not as a
  measured colour map. Marked surface reads blue; everything else keeps the
  scan's own colour, texture and lighting. The preview had been shaded through
  the deviation map's path, which drops the layer tint and the texture and
  reduces the light, so opening the brush turned both arches into a pale glossy
  shell and a textured scan lost the colour it was being read against.
- A whole-mesh command no longer repaints a scan that has nothing marked on it:
  "Fit everywhere" leaves a mask that marks nothing, and attaching it uploaded a
  whole vertex array for a picture identical to the scan.
- The status line after a Brush command names the scan it reached when the Mesh
  selection narrowed it to one, instead of stating the rule as though both had
  been changed.
- The Ruler drops a perpendicular onto a ruler line. After the first point,
  a click on a drawn ruler line ends the measurement at the foot of the
  perpendicular on that line, marked with a right angle, instead of at the
  surface under the cursor. This is the Korkhaus anterior arch length: from the
  incisal point to the line through Pont's premolar points, which runs above
  the palate. Before, the second point could only land on the scan, so the
  reading went down to the palate and was longer than the perpendicular. The
  length is measured in 3D. Hovering the line shows the perpendicular and its
  length before the click; dragging the first point or either end of the base
  line keeps the right angle.
- Pressing a ruler end without moving the mouse leaves it where it is. The
  press re-picked the end onto the surface nearest the camera under the
  pointer, so an end seen through the model (measurements draw on top of it)
  jumped to the front surface and the reading changed although nothing was
  dragged. The end now follows the pointer once it moves past the click
  tolerance.

### Files

- Saving a layer back out keeps the format it was opened in by default. Scans
  already round-tripped PLY/STL/OBJ that way, but a scan opened from a format
  with no writer (HPS, GLB, OFF) silently became a PLY, and the preference that
  produced the substitution was not stated anywhere. Settings now asks one
  question with two answers — save each scan in its own
  format, or save every scan in a chosen format — instead of a switch beside a
  format that looked like it applied either way. The format chips appear only in
  the mode that uses them, and the mode in force is always named. Whichever
  mode is chosen, a scan whose geometry cannot be written in the resulting
  format is saved as PLY rather than STL.
- A merged scene is saved in the fallback format instead of always PLY, so
  "Save scene as" follows the same preference as the layers.
- A PLY export of a textured scan writes the atlas as per-vertex RGBA, at each
  vertex's texture coordinate. PLY stores coordinates but not an image, so this
  is the only way the colour travels with the geometry. The `OccluViewTexture*`
  header comments are no longer written: a 4 MB `.dcm` could become a ~50 MB
  `.ply` that most other tools would not read. Coordinates are omitted once the
  colour is baked, because `s`/`t` with no image makes readers draw the scan
  white. A PLY an earlier release wrote with the header comment still opens in
  colour. An OBJ export of the same scan carries the colour the same way, as
  per-vertex RGB. A layer that also carries its own colours exports the atlas
  and reports that the other colours were replaced.
- A scan that names its image beside it arrives with the image. An OBJ with its
  `mtllib`/`map_Kd` pair — or with an image sharing its name — and a PLY from
  another tool with a `comment TextureFile` line are read with the texture
  attached, instead of being imported untextured and losing the colour the scan
  was captured with. A file larger than 1 GB, or a companion image larger than
  64 MB, is refused rather than read.
- A crafted PLY header can no longer make the importer hold a second copy of a
  base64 payload it was always going to reject, and a header whose
  `OccluViewTexture` keys were re-cased by a text editor reads as before,
  matching the case-insensitive treatment `TextureFile` already had.
- An HPS/DCM scan whose texture has its red and blue channels transposed is
  corrected to warm at any brightness. The check previously compared a
  brightness-scaled margin, so the same swap was caught on a dark atlas and
  missed on a bright one; a 3Shape lab scanner writes the bright kind, and
  those scans opened with cyan gingiva and blue-tinted enamel. It now measures
  the blue bias per hue-bearing pixel and requires a near-uniform bias of at
  least 24 levels on average. The sample spans rows and columns, so a
  power-of-two atlas is judged from the whole picture. A texture whose format
  is declared explicitly is decoded as declared and never re-guessed.
- The save format is no longer a setting. Settings used to ask "each scan keeps
  its own format" or "chosen format", with a format to pick in the second mode
  and two lines explaining the consequence — and the mode that was in force
  could still propose a colourless `.stl` for a scan captured in colour, leaving
  only a status-line warning after the name had been picked. That whole question
  is gone, together with the two notes under it. A scan keeps the format it was
  opened in when the viewer can write it; a scan from a format it cannot write
  is saved as PLY when it holds a texture, vertex colours or a mapping, and as
  STL when it is geometry alone. There is nothing left to choose and nothing
  left to contradict.
- Settings is shorter and the two references sit side by side. "Keyboard and
  mouse" and "About OccluView" were two full-width text lines; they are now one
  row of two equal buttons.
- A malformed binary PLY can no longer hang the viewer or the Explorer preview.
  A face element declared with rows but no property used to consume no bytes per
  row, so the reader looped forever on the same empty state; it is now refused
  with a typed error, as the ASCII reader already refused it.
- Opening several scans at once parses two of them at a time and holds at most
  half a gigabyte of file data in memory, instead of parsing every dropped file
  at once — a scan larger than that budget is parsed on its own, up to the 1 GB
  a single file may be. A bigger file is refused with a sentence in the
  operator's language that gives both sizes in gigabytes.
- The contact reading says which scan it is measured against when the scene has
  more than one candidate, and offers the others: an upper, a lower and a wax-up
  used to resolve by proximity alone, which cannot tell two similar arches
  apart. With one obvious antagonist the automatic pick stands and nothing
  changes.
- A contact measurement whose worker thread died reports a failure and offers
  "Read again" instead of leaving the bar measuring forever with no status text
  and no way out.

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
- The heatmap opens at 0.05–0.20 mm. Its cool and hot limits are editable above and below the colour legend.
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
