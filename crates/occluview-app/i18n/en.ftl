## OccluView English catalog — canonical source.
## Every key here is contract: translations must mirror key/attribute/variable sets exactly.
## House style (per humanizer review): Title Case feature names
## ("Cut View", "Mesh Editing"); … ellipsis (never ...); straight
## apostrophes; em dashes with spaces; no "please" in UI strings.

app-title = OccluView
app-window-title = OccluView 3D Viewer
align-panel-title = Align Scans
meshedit-window-title = Mesh Editing

settings-language-label = Language
settings-language-auto = System language
settings-language-auto-current = System language — { $language }
settings-language-catalog-fallback = { $tag } is not available; using English.
settings-language-save-error = Language preference could not be saved. Retrying…

about-title = About OccluView
about-tagline = Mesh Repair · Mesh Editing for dental CAD
about-version = Version { $version }

update-available-title = Update available
update-available-body = Version { $version } is ready to install.
update-current-version = You are on { $version }.
update-download = Download update
update-open-release = Open release page
update-later = Later
update-skip = Skip this version
update-skip-tooltip = Do not offer this version again; the next release will be offered
update-downloading = Downloading OccluView { $version }
update-ready-title = OccluView { $version } is ready to install
update-ready-hint-windows = The installer was verified. OccluView will close while Windows applies the update.
update-ready-hint-other = The package was verified. Your system package installer will open — confirm the update there.
update-install-close = Install and close
update-failed-title = Update failed
update-dismiss = Dismiss

error-open-title = Cannot open file
error-add-title = Cannot add file
error-open-body = Could not open { $path }.

## Help surface: section titles, control actions, contextual lines, dialog chrome.
## Gesture names (key/mouse vocabulary) stay invariant by contract.

help-title = Keyboard and mouse controls
help-subtitle = The reference below matches the controls currently available in OccluView.
help-close = Close

help-section-navigation = Navigation
help-section-tools = Tools
help-section-mesh-editing = Mesh Editing
help-section-sculpt = Sculpt
help-section-align-measure = Align and Measure
help-section-cut-view = Cut View
help-section-layers-preview = Layers and Explorer Preview

help-hintline-navigation = RMB drag orbit · MMB drag pan · Wheel zoom · MMB click focus
help-hintline-mesh-editing = LMB select · Shift+click unmark · Drag rectangle · Ctrl+Z undo
help-hintline-sculpt = LMB sculpt · Shift changes mode · Shift+wheel size · Ctrl+wheel force
help-hintline-align = LMB place · Ctrl/Command+drag rotate · Shift+drag erase · RMB undo
help-hintline-cut = LMB plant or move · Ctrl+wheel in Section resizes · F flips · Esc closes
help-hintline-measure = LMB measure · RMB clears · Wheel zooms · Esc closes

help-hint-navigation-orbit-the-camera = Orbit the camera
help-hint-navigation-pan-the-camera = Pan the camera
help-hint-navigation-pan-the-camera-2 = Pan the camera
help-hint-navigation-zoom-toward-the-pointer = Zoom toward the pointer
help-hint-navigation-recenter-on-the-surface = Recenter on the surface
help-hint-navigation-recenter-on-the-surface-when-enabled = Recenter on the surface when enabled
help-hint-navigation-open-the-layer-or-scene-menu-when-stationary = Open the layer or scene menu when stationary
help-hint-tools-open-a-file = Open a file
help-hint-tools-open-cut-view = Open Cut View
help-hint-tools-arm-the-ruler = Arm the Ruler
help-hint-tools-arm-thickness = Arm Thickness
help-hint-tools-open-align = Open Align
help-hint-tools-open-mesh-editing = Open Mesh Editing
help-hint-mesh-editing-select-a-face = Select a face
help-hint-mesh-editing-unmark-a-face-or-screen-selection = Unmark a face or screen selection
help-hint-mesh-editing-select-faces-in-a-screen-rectangle = Select faces in a screen rectangle
help-hint-mesh-editing-draw-a-freehand-selection-outline = Draw a freehand selection outline
help-hint-mesh-editing-close-and-apply-a-lasso-outline = Close and apply a lasso outline
help-hint-mesh-editing-cancel-the-active-lasso-outline = Cancel the active lasso outline
help-hint-mesh-editing-select-all-visible-faces = Select all visible faces
help-hint-mesh-editing-delete-selected-faces = Delete selected faces
help-hint-mesh-editing-undo-the-last-mesh-edit = Undo the last mesh edit
help-hint-mesh-editing-redo-the-last-mesh-edit = Redo the last mesh edit
help-hint-sculpt-choose-add-remove = Choose Add/Remove
help-hint-sculpt-choose-smooth = Choose Smooth
help-hint-sculpt-sculpt-under-the-brush = Sculpt under the brush
help-hint-sculpt-remove-or-strengthen-the-active-brush-mode = Remove or strengthen the active brush mode
help-hint-sculpt-change-brush-size = Change brush size
help-hint-sculpt-change-brush-intensity = Change brush intensity
help-hint-align-measure-place-an-alignment-point-or-measurement-point = Place an alignment point or measurement point
help-hint-align-measure-rotate-a-scan-in-align-s-manual-mode = Rotate a scan in Align's Manual mode
help-hint-align-measure-erase-an-align-exclusion-region = Erase an Align exclusion region
help-hint-align-measure-change-align-exclusion-brush-size = Change Align exclusion-brush size
help-hint-align-measure-undo-the-last-alignment-point-when-stationary = Undo the last alignment point when stationary
help-hint-align-measure-clear-measurements-when-stationary = Clear measurements when stationary
help-hint-align-measure-close-the-active-measurement-tool = Close the active measurement tool
help-hint-cut-view-plant-or-move-the-cut-disc = Plant or move the cut disc
help-hint-cut-view-change-disc-size = Change disc size
help-hint-cut-view-zoom-the-section-view = Zoom the section view
help-hint-cut-view-flip-the-kept-half-while-planted = Flip the kept half while planted
help-hint-cut-view-unplant-the-disc-or-close-cut-view = Unplant the disc or close Cut View
help-hint-layers-preview-hide-the-layer-under-the-pointer = Hide the layer under the pointer
help-hint-layers-preview-restore-the-last-hidden-layer = Restore the last hidden layer
help-hint-layers-preview-toggle-layer-translucency = Toggle layer translucency
help-hint-layers-preview-orbit-the-preview-model = Orbit the preview model
help-hint-layers-preview-zoom-the-preview-model = Zoom the preview model
help-hint-layers-preview-frame-the-preview-model = Frame the preview model
help-hint-layers-preview-toggle-preview-wireframe = Toggle preview wireframe

## Toolbar, empty state. Shortcut glyphs interpolate as variables and stay invariant.

toolbar-open-label = Open
toolbar-open-hint = Open 3D files ({ $shortcut })
toolbar-recent-hint = Recent files
toolbar-add-label = Add
toolbar-add-hint = Add more files to the current scene
toolbar-cut-label = Cut View
toolbar-cut-hint = Slice the model along a plane ({ $shortcut })
toolbar-cut-unavailable = Cut View needs a visible layer
toolbar-ruler-label = Ruler
toolbar-ruler-hint = Measure a distance: click two points on the model ({ $shortcut })
toolbar-thickness-label = Thickness
toolbar-thickness-hint = Probe the local wall thickness: click a point on the shell ({ $shortcut })
toolbar-measure-blocked = Finish or cancel the mesh edit session first
toolbar-measure-needs-layer = Measuring needs a visible mesh layer
toolbar-align-label = Align
toolbar-align-hint = Bring two scans together: click a point on each ({ $shortcut })
toolbar-edit-label = Edit
toolbar-edit-open = Mesh Editing is open
toolbar-edit-hint = Mesh Editing: selection and sculpting ({ $shortcut })
toolbar-settings-label = Settings
toolbar-settings-hint = Open preferences

empty-open-file = Open a 3D file
empty-formats-hint = STL · PLY · OBJ · GLB · HPS — or drop files here

## Loading and export. Counts are numeric selects; paths stay raw data.

load-queued = { $count ->
    [one] Queued { $count } layer
   *[other] Queued { $count } layers
}
load-opening = { $count ->
    [one] Opening { $count } file…
   *[other] Opening { $count } files…
}
load-adding = { $count ->
    [one] Adding { $count } file…
   *[other] Adding { $count } files…
}
load-open-failed-start = Open failed: could not start loader
load-add-failed-start = Add failed: could not start loader
load-open-failed-stopped = Open failed: loader stopped
load-loader-failed-summary = The background scene loader could not be started.
load-action-failed-open = Open failed: { $detail }
load-action-failed-add = Add failed: { $detail }

export-nothing-visible = Nothing visible to save
export-unsupported-format = Unsupported output format
export-scene-saved = Scene saved: { $path }
export-scene-saved-unmerged = Scene saved (textures are not merged): { $path }
export-scene-failed-title = Could not save the scene
export-scene-failed-summary = Could not save the scene: { $detail }
export-layers-saved = { $written ->
    [one] Saved { $written } layer to { $dir }
   *[other] Saved { $written } layers to { $dir }
}
export-layers-saved-failed = { $written ->
    [one] Saved { $written } layer to { $dir }
   *[other] Saved { $written } layers to { $dir }
}; { $failed ->
    [one] { $failed } could not be written
   *[other] { $failed } could not be written
}
export-layers-saved-renamed = { $written ->
    [one] Saved { $written } layer to { $dir }
   *[other] Saved { $written } layers to { $dir }
}; { $renamed ->
    [one] { $renamed } file renamed to keep what was already there
   *[other] { $renamed } files renamed to keep what was already there
}
export-layers-saved-failed-renamed = { $written ->
    [one] Saved { $written } layer to { $dir }
   *[other] Saved { $written } layers to { $dir }
}; { $failed ->
    [one] { $failed } could not be written
   *[other] { $failed } could not be written
}; { $renamed ->
    [one] { $renamed } file renamed to keep what was already there
   *[other] { $renamed } files renamed to keep what was already there
}
mesh-exported-aligned = Exported { $name } in its aligned position as { $format }: { $path }
mesh-exported-aligned-warnings = Exported { $name } in its aligned position as { $format } (warnings: { $warnings }): { $path }
mesh-exported-unmoved = Exported { $name } (this scan has not been moved) as { $format }: { $path }
mesh-exported-unmoved-warnings = Exported { $name } (this scan has not been moved) as { $format } (warnings: { $warnings }): { $path }
mesh-warning-vertex-colors = vertex colors not included
mesh-warning-uvs = UVs not included
mesh-warning-texture-image = texture image not included
mesh-export-warnings = Export warnings: { $warnings }
mesh-export-failed-title = Could not export layer
mesh-export-failed-summary = Could not export layer: { $detail }

## Repair card and toasts. `$count` drives the select, `$grouped` shows
## thousands-grouped digits (a grouped string cannot plural-match).

repair-title = Mesh Repair
repair-clean-headline = Nothing to repair — mesh is clean
repair-copy-details = Copy details
repair-copy-tooltip = Copy the full per-pass report to the clipboard
repair-line-welded = { $count ->
    [one] Welded { $grouped } duplicate vertex
   *[other] Welded { $grouped } duplicate vertices
}
repair-line-slivers = { $count ->
    [one] Removed { $grouped } sliver face
   *[other] Removed { $grouped } sliver faces
}
repair-line-duplicate-faces = { $count ->
    [one] Removed { $grouped } duplicate face
   *[other] Removed { $grouped } duplicate faces
}
repair-line-nonmanifold = { $count ->
    [one] Fixed { $grouped } non-manifold edge
   *[other] Fixed { $grouped } non-manifold edges
}
repair-line-bowtie = { $count ->
    [one] Split { $grouped } bowtie vertex
   *[other] Split { $grouped } bowtie vertices
}
repair-line-reoriented = { $count ->
    [one] Reoriented { $grouped } triangle
   *[other] Reoriented { $grouped } triangles
}
repair-line-flipped = { $count ->
    [one] Flipped { $grouped } inside-out part
   *[other] Flipped { $grouped } inside-out parts
}
repair-line-debris = { $count ->
    [one] Removed { $grouped } debris part
   *[other] Removed { $grouped } debris parts
}
repair-line-pinholes = { $count ->
    [one] Closed { $grouped } pinhole
   *[other] Closed { $grouped } pinholes
}
repair-line-unused = { $count ->
    [one] Removed { $grouped } unused vertex
   *[other] Removed { $grouped } unused vertices
}
repair-open-rims = { $count ->
    [one] { $grouped } open rim left (scan boundary)
   *[other] { $grouped } open rims left (scan boundary)
}
repair-skipped-rims = { $count ->
    [one] { $grouped } rim could not be filled (non-simple)
   *[other] { $grouped } rims could not be filled (non-simple)
}
repair-toast-welded = { $count ->
    [one] welded { $count } vertex
   *[other] welded { $count } vertices
}
repair-toast-slivers = { $count ->
    [one] removed { $count } sliver
   *[other] removed { $count } slivers
}
repair-toast-duplicate-faces = { $count ->
    [one] { $count } duplicate face
   *[other] { $count } duplicate faces
}
repair-toast-nonmanifold = { $count ->
    [one] fixed { $count } non-manifold edge
   *[other] fixed { $count } non-manifold edges
}
repair-toast-bowtie = { $count ->
    [one] split { $count } bowtie
   *[other] split { $count } bowties
}
repair-toast-reoriented = { $count ->
    [one] reoriented { $count } triangle
   *[other] reoriented { $count } triangles
}
repair-toast-flipped = { $count ->
    [one] flipped { $count } inside-out part
   *[other] flipped { $count } inside-out parts
}
repair-toast-debris = { $count ->
    [one] dropped { $count } debris part
   *[other] dropped { $count } debris parts
}
repair-toast-pinholes = { $count ->
    [one] closed { $count } pinhole
   *[other] closed { $count } pinholes
}
repair-toast-unused = { $count ->
    [one] removed { $count } unused vertex
   *[other] removed { $count } unused vertices
}
repair-toast-skipped = { $count ->
    [one] { $count } rim skipped (non-simple)
   *[other] { $count } rims skipped (non-simple)
}
repair-toast-done = Repaired { $layer }: { $parts }
repair-toast-clean-rims = Mesh is already clean: { $layer }, { $count ->
    [one] { $count } open rim left
   *[other] { $count } open rims left
}
repair-toast-clean = Mesh is already clean: { $layer }
repair-edit-busy = Layer edit already in progress
repair-edit-failed-title = Could not edit layer
repair-edit-failed-summary = Could not edit layer: { $detail }
edit-locked-status = { $status } (not undoable: snapshot too large)

## Layers overlay, layer menu, scene menu.

layers-title = Layers
layers-count = { $count ->
    [one] { $count } layer
   *[other] { $count } layers
}
layers-row-hide = Hide layer
layers-row-show = Show layer
layers-row-opacity = Layer opacity
layers-row-remove = Remove layer

layer-menu-next-tint = Next tint
layer-menu-hide-colors = Hide scan colors
layer-menu-show-colors = Show scan colors
layer-menu-disable-texture = Disable texture
layer-menu-show-texture = Show texture
layer-menu-mesh-editing = Mesh Editing
layer-menu-split-bridge = Split bridge…
layer-menu-repair = Mesh Repair
layer-menu-flip-normals = Flip normals
layer-menu-export = Export layer…
layer-menu-hide-wireframe = Hide wireframe
layer-menu-show-wireframe = Wireframe overlay
layer-menu-remove = Remove layer

scene-menu-title = Scene
scene-menu-save = Save scene as…
scene-menu-save-each = Save each layer…
scene-menu-reset = Reset positions
scene-menu-fit = Fit view

## Mesh editor palette.

meshedit-tab-edit = Mesh Editing
meshedit-tab-sculpt = Sculpt
meshedit-cancel-session = Cancel the session (edits are reverted)
meshedit-header-edit = Mesh Editing
meshedit-section-selection = Selection
meshedit-section-edit-selection = Edit selection
meshedit-section-close-holes = Close holes
meshedit-section-sculpt = Sculpt
meshedit-cell-lasso = Lasso
meshedit-cell-lasso-hint = Freehand outline: click to place points, double-click to close · Shift unmarks
meshedit-cell-object = Object
meshedit-cell-object-hint = Click a whole object of a multi-part STL to select it · Shift unmarks
meshedit-cell-surface = Surface
meshedit-cell-surface-hint = Mark only the visible front-facing surface
meshedit-cell-through = Through
meshedit-cell-through-hint = Mark straight through the mesh, including hidden backsides
meshedit-cell-all = All
meshedit-cell-all-hint = Mark every face (Ctrl+A)
meshedit-cell-none = None
meshedit-cell-none-hint = Clear the marking
meshedit-cell-invert = Invert
meshedit-cell-invert-hint = Swap marked and unmarked faces
meshedit-cell-delete = Delete
meshedit-cell-delete-hint = Delete the marked faces
meshedit-cell-crop = Crop
meshedit-cell-crop-hint = Keep only the marked area, remove the rest
meshedit-cell-cut = Cut
meshedit-cell-cut-hint = Move the marked faces to a new mesh — the original stays put
meshedit-cell-separate = Separate
meshedit-cell-separate-hint = Split the marked region into one mesh per connected part
meshedit-cell-close-holes = Close holes
meshedit-cell-close-holes-hint = Close holes only when the surrounding faces are selected. Scan borders stay open.
meshedit-sculpt-addremove = Add / Remove  [1]
meshedit-sculpt-addremove-hint = Build material up by dragging on the scan; hold Shift to carve it away. Shift+wheel resizes, Ctrl+wheel changes intensity. Hotkey: 1.
meshedit-sculpt-smooth = Smooth  [2]
meshedit-sculpt-smooth-hint = Relax the surface by dragging on the scan; hold Shift to force maximum smoothing. Shift+wheel resizes, Ctrl+wheel changes intensity. Hotkey: 2.
meshedit-slider-size = size
meshedit-slider-size-hint = Brush size (Shift + mouse wheel)
meshedit-slider-force = force
meshedit-slider-force-hint = Brush intensity (Ctrl + mouse wheel)
meshedit-limit-label = limit
meshedit-limit-checkbox-hint = Restrict repair to rims no larger than this perimeter
meshedit-limit-drag-hint = Off closes every safe hole inside the selected area; the scan border stays open
meshedit-status-unsaved = Unsaved edits
meshedit-status-unsaved-hint = Uncommitted edits: Done to apply, Cancel to revert
meshedit-status-hint-sculpt = Drag on the surface to sculpt · RMB orbits
meshedit-status-hint-object = Click an object to select it whole · Shift unmarks
meshedit-status-hint-lasso = Click to outline · double-click closes · Shift unmarks
meshedit-status-hint-default = Drag a box to mark · Shift to unmark · Del deletes
meshedit-session-undo = Undo
meshedit-session-undo-hint = Undo the last mesh edit (Ctrl+Z)
meshedit-session-redo = Redo
meshedit-session-redo-hint = Redo the undone mesh edit (Ctrl+Y)
meshedit-session-cancel = Cancel
meshedit-session-cancel-hint = Discard every edit from this session
meshedit-session-done = Done
meshedit-session-done-hint = Apply the edits and close the editor

## Align Scans window.

align-title = Align Scans
align-tab-auto = Automatically
align-tab-manual = Manually
align-constraint-free = Move/rotate in all directions
align-constraint-free-hint = Drag the scan in any direction
align-constraint-z = Move in z-direction
align-constraint-z-hint = Drag only along the vertical axis
align-constraint-xy = Move in xy-plane
align-constraint-xy-hint = Drag only across the horizontal plane
align-manual-drag-hint = Drags whichever scan you grab · Ctrl+drag turns it
align-undo = Undo
align-undo-hint = Go back one step
align-redo = Redo
align-redo-hint = Go forward one step
align-prompt-moving = Click a point on the mesh that should move
align-prompt-other = Click the same position on the other mesh
align-prompt-alternate = Click alternating points at the same positions on the two meshes
align-prompt-placed = { $count ->
    [one] { $count } arrow placed
   *[other] { $count } arrows placed
}
align-back = Back
align-back-hint = Undo an arrow — a right-click in the view does the same
align-clear = Clear
align-clear-hint = Drop every arrow and pick two scans again — the scans stay where they are
align-fit-perform = Perform alignment
align-fit-perform-hint = Move the mesh onto the arrows — needs at least two arrows
align-fit-refine = Best fit matching
align-fit-refine-hint = Match unchanged areas of a prepared scan to the original model. Check the result before accepting it
align-matching-parts = matching parts
align-matching-parts-hint = Maximum share of surface matches kept during refinement. Best fit can lower it when few areas are unchanged
align-max-influence = max influence
align-max-influence-hint = Only surface below this distance influences the matching. A large value can worsen the result
align-orientation-title = Surfaces orientation shall match
align-orientation-match = Surfaces orientation shall match
align-orientation-inverted = Surfaces orientation shall match inverted
align-orientation-ignored = Surfaces orientation shall be ignored
align-orientation-either-hint = Accepts either facing. The calculation often takes significantly longer
align-orientation-facing-hint = How the two surfaces are taken to face each other
align-exclude = Matching: Exclude selected parts
align-exclude-hint = Paint the surface best-fit matching must ignore
align-commit-cancel = Cancel
align-commit-cancel-hint-moved = Put every scan back where it was and close — Ctrl+Z brings the alignment back
align-commit-cancel-hint-clean = Close without changing anything
align-commit-done = Done
align-commit-done-hint = Keep the alignment and close — export the scan to write it to disk

## Deviation map. All clinical numbers arrive pre-formatted as strings.

align-map-heatmap = Heatmap
align-map-heatmap-hint = Colour one scan by how far it sits from the other
align-map-requires-refine = Run Best fit matching first
align-map-max = max
align-map-not-measured = not measured
align-map-not-measured-hint = No surface on the other scan within reach of these vertices. A tooth or a bridge that only one scan has is the usual reason, and it is not an error — there is nothing there to measure to.

## Align roles, brush, mask commands, align status lines.

align-pair-decided = { $moving } → { $fixed }
align-pair-guessed = { $moving } → { $fixed } (a guess)
align-pair-hint-decided = { $moving } moves, { $fixed } stays put
align-pair-hint-guessed = Nothing clicked yet, so the tool guessed from the order the files were opened. Your first click decides it: { $moving } moves, { $fixed } stays put
align-pair-swap = Swap
align-pair-swap-hint = Fit the other way round — the arrows move with it

align-brush-title = Brush tool
align-brush-close-hint = Close the brush — the markings are kept
align-brush-mesh-selection = Mesh selection
align-brush-moving = Moving
align-brush-fixed = Fixed
align-brush-size = brush size
align-brush-inverse = Brush inverse
align-brush-inverse-hint = A plain drag clears instead of marks. Shift inverses it again
align-brush-auto-radius = automatic radius
align-brush-auto-radius-hint = The radius of the mesh area kept at each arrow end
align-brush-size-status = Brush { $size } mm
align-status-no-summary = No comparable surface

align-mask-fit-everywhere = Fit everywhere
align-mask-fit-everywhere-hint = Clear all existing markings
align-mask-fit-everywhere-report = Markings cleared — matching on the whole scan
align-mask-fit-nowhere = Fit nowhere
align-mask-fit-nowhere-hint = Mark the complete mesh — best-fit matching will have no effect
align-mask-fit-nowhere-report = Whole mesh marked — best-fit matching will have no effect
align-mask-invert = Invert markings
align-mask-invert-hint = Mark unmarked areas and vice versa
align-mask-invert-report = Markings inverted
align-mask-automatic = Mark automatic
align-mask-automatic-hint = Match only on a small area around each arrow end
align-mask-automatic-report = Matching only around the arrow ends

align-status-half-dropped = Half-placed arrow dropped
align-status-turned = Pair turned around
align-status-cleared = Pair cleared
align-status-click-moving = Click a point on the scan that should move
align-status-click-alternate = Click alternating points at the same positions on the two meshes
align-status-two-scans = Two scans in view — click a point on each to pair them
align-status-no-surface = A point cloud has no surface to pair
align-status-now-other = Now click the matching spot on the other scan
align-status-moved = Point moved
align-status-wrong-scan = That scan is not in this pair — press Clear to start over
align-status-one-scan = One of the scans
align-status-place-first = Place a point on each scan first
align-status-scaled = That scan carries a scaled placement, which cannot be aligned
align-status-pose-refused = The fit finished, but the scan it was for is no longer available
align-status-worker-unavailable = Alignment worker stopped — restart the alignment tool
align-status-measure-dropped = Measurement dropped — the marking brush owns the colours
align-status-measure-unavailable = Measurement not applied — the scan changed; run Best fit matching again
align-status-map-elsewhere = Distance map is on the Automatically tab — it comes back there
align-status-aligned-points = Aligned on points

## Align result status lines. Clinical numbers arrive pre-formatted.

align-status-aligned = Aligned on points — run Best fit matching to seat the surfaces.
align-status-refined = Best fit ready
align-status-measured = Heatmap updated
align-status-remeasure = { $reason } — run Best fit matching to measure again
align-status-settings-changed = Matching settings changed
align-status-visibility-changed = Selected scan visibility changed
align-brush-not-in-alignment = That mesh is not in this alignment
align-drag-moving = Moving { $name } by hand
align-drag-unrecorded = Moved by hand, but this step could not be added to the history — Ctrl+Z will not undo it
align-drag-moved = { $name } moved { $moved } mm by hand (Ctrl+Z undoes)
align-status-moved-hand = Moved by hand
align-pair-placed = Pair { $n } placed
align-roles-swapped = { $moving } moves now, { $fixed } stays put
align-status-scan-changed = The scan changed
align-status-hidden = { $name } is hidden — show it to align against it
align-arrow-removed = { $n ->
    [one] Arrow removed — { $n } pair left
   *[other] Arrow removed — { $n } pairs left
}
align-status-markings-changed = Markings changed
align-status-place-arrow-first = Place at least one arrow before marking automatically
align-status-arrows-cleared = Arrows cleared — moving by hand from here

## Unsaved-work guards and error dialog buttons.

guard-close-title = Unsaved mesh edits
guard-close-headline-one = 1 edited layer has not been saved to disk.
guard-close-headline-many = Edited layers have not been saved to disk.
guard-close-note = { $count } edited layers are affected.
guard-close-detail = Save exports each edited layer (PLY, STL, or OBJ) and then closes.
guard-close-destructive = Close without saving
guard-replace-title = Edit in progress
guard-replace-headline-session = An edit session is active on { $layer }.
guard-replace-headline-one = 1 edited layer has unsaved changes.
guard-replace-headline-many = { $count } edited layers have unsaved changes.
guard-replace-detail = Opening a scene closes the session and discards edits not saved to disk.
guard-replace-destructive = Discard and open
guard-save = Save…
guard-cancel = Cancel

error-retry-graphics = Try again
error-close = Close
error-copy-details = Copy Details

about-website = Website
about-source = Source
about-licenses = Third-party licenses
about-license-kind = Apache License 2.0

## Mesh-edit operations, undo/redo, sculpt, measure, cut ruler, scene menu.

edit-select-faces-first = Select mesh faces first
edit-no-changes = No changes: { $layer }
edit-apply-failed-title = Could not edit selection
edit-apply-failed-summary = Could not edit selection: { $detail }
edit-no-changes-hidden = No changes: refine the selection; hidden layers stay untouched
edit-selected-faces = { $faces ->
    [one] Selected { $faces } face
   *[other] Selected { $faces } faces
}
edit-selected-faces-across = { $faces ->
    [one] Selected { $faces } face across { $layers } layers
   *[other] Selected { $faces } faces across { $layers } layers
}

holes-nothing = No holes to close: { $layer }
holes-partial = { $segments }, none closed: { $layer }
holes-closed = { $filled ->
    [one] Closed { $filled } hole
   *[other] Closed { $filled } holes
}
holes-closed-detail = { $closed }: { $layer }
holes-closed-segments = { $closed } ({ $segments }): { $layer }
holes-seg-healed = { $n ->
    [one] { $n } nick healed
   *[other] { $n } nicks healed
}
holes-seg-border = scan border kept open
holes-seg-oversize-limit = { $n ->
    [one] { $n } hole over the { $limit } mm limit
   *[other] { $n } holes over the { $limit } mm limit
}
holes-seg-oversize = { $n ->
    [one] { $n } hole too large
   *[other] { $n } holes too large
}
holes-seg-damaged = { $n ->
    [one] { $n } damaged rim skipped
   *[other] { $n } damaged rims skipped
}
batchedit-close-holes = Closed safe interior holes
batchedit-delete = Deleted selection
batchedit-crop = Cropped selection
batchedit-cut = Cut selection to new layer
batchedit-separate = Separated selection
batchedit-invert = Inverted normals
batch-close-holes = Closed safe interior holes
batch-delete = Deleted selection
batch-crop = Cropped selection
batch-cut = Cut selection
batch-separate = Separated selection
batch-edited = Edited selection
batchedit-edited = Edited layer
edit-applied-status = { $action }: { $layer }
batchedit-status = { $label } on { $n ->
    [one] { $n } visible layer
   *[other] { $n } visible layers
}
batchedit-no-changes = No changes: refine the selection; hidden layers stay untouched

select-covers-all = Selection already covers the whole mesh: { $layer }
select-covers-remove = Selection covers the whole mesh — remove the layer instead: { $layer }
select-splits = Selection splits into { $parts } parts — refine the selection: { $layer }
select-faces-cannot = Cannot select faces: { $layer }

undo-nothing = Nothing to undo
redo-nothing = Nothing to redo
undo-undid = Undid mesh edit: { $layer }
undo-unavailable = Undo unavailable — the scene changed since that step: { $layer }
redo-redid = Redid mesh edit: { $layer }
redo-unavailable = Redo unavailable — the scene changed since that step: { $layer }

sculpt-armed-addremove = Add/Remove: drag to build, hold Shift to carve
sculpt-armed-smooth = Smooth: drag to relax, hold Shift to force it
sculpt-off = Sculpt off
sculpt-applied-undo = Sculpt applied (Ctrl+Z undoes)
sculpt-applied-locked = Sculpt applied (not undoable: snapshot too large)
sculpt-failed-title = Sculpt failed
sculpt-failed = Cannot sculpt this layer: { $detail }
sculpt-worker-stopped = Sculpt worker stopped: { $detail }
sculpt-preparing = Preparing sculpt brush…
sculpt-nonuniform-scale = Sculpting requires a uniformly scaled mesh
sculpt-failure-worker-panicked = Sculpt worker panicked: { $detail }
sculpt-failure-spawn = Could not start sculpt worker: { $detail }
sculpt-failure-kernel-pool = Could not create sculpt kernel pool: { $detail }
sculpt-failure-missing-undo-baseline = Sculpt stroke has no undo baseline
sculpt-failure-shadow-poisoned = Sculpt shadow lock was poisoned
sculpt-failure-shadow-shape = Sculpt display shadow no longer matches the live mesh
sculpt-failure-invalid-vertex-index = Sculpt worker returned an invalid vertex index
sculpt-failure-worker-state-poisoned = Sculpt worker state was corrupted — restart Sculpt
sculpt-failure-vertex-count-changed = Sculpt result changed the vertex count
sculpt-failure-topology-rebuild = Sculpt topology rebuild failed: { $detail }
sculpt-worker-unavailable = Sculpt worker is unavailable
sculpt-finishing = Finishing sculpt stroke…
sculpt-finishing-history = Finishing sculpt before history change…
sculpt-lasso-armed = Lasso armed: click or drag to outline; Enter, double-click, or click the start closes
sculpt-lasso-off = Lasso disarmed
sculpt-object-on = Object select: click an object to select it whole
sculpt-object-off = Object select off
sculpt-selection-cleared = Selection cleared
sculpt-through-on = Through-mesh selection
sculpt-through-off = Surface selection

## Session close-outs and layer shortcuts.
session-applied = Mesh Editing session applied
session-reverted = Mesh Editing session reverted
edit-session-busy = Finish or cancel mesh editing first
layers-none-hidden = No hidden layers to restore
layer-opaque-again = Opaque again: { $label }
layer-translucent = Translucent: { $label } (Shift+Middle click restores)
layer-restored = Visible again: { $label }
layer-hidden = Hidden: { $label } (Shift+Ctrl+Middle click restores)
layer-unnamed = layer { $n }
layer-removed = Removed layer: { $label }
layer-face-selection = Face selection: { $label }

measure-distance = Distance: { $len }
measure-thickness = Wall thickness: { $len }
measure-open-wall = Open surface: no opposite wall along the inward normal
measure-cannot-probe = Cannot probe here: degenerate surface geometry
measure-cleared = Measurements cleared

cut-lines = Lines
cut-mesh = Mesh
cut-dist = Dist
cut-dist-hint = Distance: click two points
cut-thick = Thick
cut-thick-hint = Wall thickness: click one point on the contour
cut-close-section = Close section
cut-snap = Snap
cut-snap-hint = Magnet: click points snap to the section contour
cut-empty = No intersection
cut-footer-distance = Drag = pan · click 2 pts = distance · right-click clears · scroll = zoom
cut-footer-thickness = Drag = pan · click contour = wall thickness · right-click clears · scroll = zoom

## Bridge split panel and align session close-outs.

bridge-panel-title = Bridge split
bridge-mode-place = Place disc
bridge-mode-calculating = Calculating
bridge-mode-ready = Ready
bridge-mode-failed = Split attempt failed
bridge-kerf = Kerf
bridge-disc-size = Disc size
bridge-cancel = Cancel
bridge-apply = Split bridge
bridge-err-miss = Disc misses the bridge. Move it into a connector.
bridge-err-tangent = Disc only touches the surface. Move it through the connector.
bridge-err-small = Disc diameter is { $have } mm; at least { $need } mm is needed here.
bridge-err-limit = This cut needs a { $need } mm disc, above the { $max } mm safety limit.
bridge-err-no-result = The split was attempted with the source surface preserved, but no usable result was produced. The original mesh was kept.
bridge-err-invalid-cut = The split was attempted, but the resulting cut could not be validated. The original mesh was kept.
bridge-err-invalid-side = The split was attempted, but { $side } could not be validated. The original mesh was kept.
bridge-err-gap = The split was attempted, but the requested gap could not be preserved. The original mesh was kept.
bridge-err-empty = The selected layer has no triangle mesh to split.
bridge-err-invalid = Disc settings are invalid. Reset the tool and try again.
bridge-err-unusable = The split could not produce a usable result. The original mesh was kept.

align-session-canceled = Alignment cancelled — every scan is back where it was (Ctrl+Z brings it back)
align-session-closed = Alignment closed
align-session-closed-running = Alignment closed — a fit was still running and was dropped, so the scans are exactly as you last saw them
align-session-kept = Alignment kept — save the scan to keep it on disk

recent-clear = Clear recent

scene-already-origin = Every layer is already at its original position
scene-positions-reset = Layer positions reset (Ctrl+Z undoes)

## Settings panel, bridge split, render error, tint.

settings-header = Settings
settings-section-files = Files & export
settings-export-format = Fallback export format
settings-export-format-hint = Used when the source format cannot be exported
settings-remember-export = Remember export folder
settings-remember-export-hint = Use the same folder after restarting OccluView
settings-section-scene = View & navigation
settings-frame-on-open = Frame a scene when it opens
settings-frame-on-open-hint = Reset the camera to the home view when a new file replaces the scene, instead of keeping the current one
settings-double-click = Double-click refocuses view
settings-double-click-hint = A double primary click re-centers the camera on the picked point
settings-orbit = Orbit speed
settings-orbit-hint = How fast the view orbits while the right mouse button drags
settings-zoom = Zoom speed
settings-zoom-hint = How much each scroll notch zooms
settings-background = Background
settings-bg-gray = Gray
settings-bg-white = White
settings-bg-dark = Dark
settings-ghost = Ghost the cut-away side
settings-ghost-hint = During a cut view, show the removed side as a translucent ghost
settings-measurements = Measurements
settings-section-appearance = Appearance
settings-theme = Theme
settings-theme-light = Light
settings-theme-dark = Dark
settings-scale = UI scale
settings-scale-hint = Scales every element; 1.0 keeps the platform default
settings-section-mesh = Mesh Editing
settings-remember-brush = Remember sculpt brush
settings-remember-brush-hint = Keep the size and intensity sliders between sessions instead of resetting them
settings-section-updates = Updates
settings-check-auto = Check automatically at startup
settings-check-now = Check now
settings-check-disabled-hint = Update checks are disabled by the environment
settings-check-busy-hint = An update check is already running
settings-update-disabled = Disabled by environment
settings-update-checking = Checking…
settings-update-current = Up to date
settings-update-skipped = Version skipped
settings-update-failed = Couldn't check
settings-save-error = Preferences could not be saved. Retrying…
settings-save-error-hint = The settings file is currently unavailable
settings-shortcuts = Keyboard shortcuts
settings-about = About OccluView

bridge-busy = Finish or cancel Bridge split first
bridge-active = Bridge split is already active
bridge-target-gone = Bridge split target is no longer available
bridge-needs-mesh = Bridge split requires a visible triangle mesh
bridge-place-disc = Bridge split: place separator disc
bridge-canceled-scene = Bridge split canceled: scene closed
bridge-canceled-camera = Bridge split canceled: camera unavailable
bridge-canceled-changed = Bridge split canceled: source mesh changed
bridge-canceled = Bridge split canceled
bridge-calculating = Bridge split: calculating
bridge-unavailable = Bridge split is temporarily unavailable
bridge-preview-stale = Bridge split preview is no longer valid
bridge-not-applied = Bridge split was not applied
bridge-complete = Bridge split complete
bridge-complete-surface = Bridge split complete (surface result; natural borders preserved)
bridge-complete-locked = Bridge split complete (not undoable: snapshot too large)

render-failed-title = Could not render scene
render-failed-summary = The file opened, but the viewport could not be rendered.
render-failed-status = Render failed

tint-choose = Choose tint

## Status tail: brush, lasso, loading, GPU, align jobs.
brush-no-mesh = Click a point on each mesh first, then paint on either
lasso-dropped = Lasso outline dropped
lasso-needs-points = Lasso needs at least 3 points
loading-scene = Loading scene…
gpu-failed-status = Graphics driver reported a problem
gpu-retry-status = Retrying graphics — if the problem persists, save your work and restart OccluView
gpu-failed-title = Graphics problem
gpu-failed-summary = The graphics driver reported a problem while drawing. The view may be incomplete. Saving your work and restarting OccluView is recommended if it keeps happening.
align-job-align = Aligning…
align-job-refine = Refining…
align-job-measure = Measuring…
align-markings-dropped = Markings dropped — the scan's surface changed since they were painted

## Worker-built align failures. The worker thread has no locale, so refusal
## details arrive pre-formatted in positional `$a`/`$b` (documented per key).

align-fail-no-surface-fixed = The fixed scan has no usable surface
align-fail-no-surface-moving = The moving scan has no usable surface
align-fail-recolor = The measurement was dropped before it could be coloured
align-fail-unobservable = The surface is not observable enough for a reliable heatmap
align-reject-toofew = Place more matching arrows or move the scans closer
align-reject-unpaired = Complete both sides of each matching arrow
align-reject-degenerate-plain = Spread the matching points across the surface
align-reject-unit = The scans use different units
align-reject-apart = Check the matching arrows and move the scans closer
align-reject-runaway = Move the scans closer and try Best fit matching again
align-reject-no-improvement = Best fit could not confirm an improvement — move the scans closer and try again
align-reject-ambiguous = Best fit found more than one equally plausible surface — mark the matching area or place the scans closer
align-reject-nonfinite = The selected point or surface is invalid
align-status-stepped = Stepped through history
align-status-moving-hand = Moving by hand

# ---------------------------------------------------------------------------
# Occlusal contacts: right-click a scan and read where it meets the scan it
# bites against. One reading is articulating paper (marks only, coloured by
# depth); the other is the approach map (how close, everywhere). One slider
# moves the depth the ramp calls fully loaded, and it re-colours a measurement
# already in hand rather than re-measuring.
# ---------------------------------------------------------------------------
layer-menu-contacts = Show contacts
layer-menu-hide-contacts = Hide contacts

contact-title = Occlusal contacts
contact-close-hint = Close the reading and take the marks off both scans
contact-against = { $subject } against { $antagonist }
contact-unknown-layer = a scan that is no longer open

contact-mode-marks = Contacts
contact-mode-marks-hint = Where the surfaces meet, coloured by how hard — the rest stays bare, as articulating paper leaves it
contact-mode-approach = Approach
contact-mode-approach-hint = How close the other scan is everywhere, load included

contact-load-label = heavy at
contact-load-suffix = mm
contact-load-hint = The depth this ramp reads as fully loaded. Moving it recolours the map already measured — no re-measurement.
contact-flatten = One colour per contact
contact-flatten-hint = Flatten every contact patch to its deepest point. Off keeps the force distribution inside each mark.

contact-legend-deepest = { $mm } mm into the bite

contact-stats-area = Contact area
contact-stats-contacts = Contacts
contact-stats-deepest = Deepest

contact-readout-gap = gap
contact-readout-load = load

contact-status-measuring = Measuring…
contact-status-measuring-hint = Reading the two surfaces against each other
contact-status-remeasuring = Re-measuring…
contact-status-remeasuring-hint = A scan moved, so the distances changed. The map is being read again.
contact-status-needs-second = A contact reading needs a second visible scan to measure against
contact-status-needs-second-hint = Open the opposing scan, or show it again, and start the reading
contact-status-subject-unusable = The scan this reading is about cannot be measured right now
contact-status-subject-unusable-hint = Show it again, or leave it as a triangle mesh, and the reading resumes
contact-status-antagonist-unusable = The scan this reading is measured against cannot be measured right now
contact-status-antagonist-unusable-hint = Show it again, or leave it as a triangle mesh, and the reading resumes
contact-status-no-overlap = The two scans are too far apart to read
contact-status-no-overlap-hint = Nothing on either surface came within the reading's reach. Check that the scans are in occlusion.
contact-status-no-surface = One of the two scans has no surface to measure
contact-status-worker-failed = The measurement did not complete
contact-status-failed-hint = Read again; if it keeps failing, the pair may need repairing first.
contact-retry = Read again

contact-opened = Reading contacts on { $label }
contact-closed = Contact reading closed
help-section-contacts = Occlusal Contacts
help-hintline-contacts = Right-click a layer · Show contacts · drag Heavy at to repaint · Esc closes
help-hint-contacts-read-its-occlusal-contacts-against-the-scan-it-bites = Read its occlusal contacts against the scan it bites against
help-hint-contacts-read-the-contact-depth-under-the-cursor = Read the contact depth under the cursor, on either arch
help-hint-contacts-move-the-depth-the-ramp-calls-fully-loaded = Move the depth the ramp calls fully loaded
help-hint-contacts-switch-between-marks-only-and-the-whole-approach = Switch between marks only and the whole approach
help-hint-contacts-close-the-reading-and-take-the-marks-off-both-scans = Close the reading and take the marks off both scans
contact-legend-gap = gap up to { $mm } mm
contact-stats-balance = Area each side
contact-stats-balance-hint = Contact area either side of this scan's own mid-line. The split follows the scan's coordinates, so a rotated or mirrored case can swap the two numbers.
layer-menu-contacts-unavailable = A contact reading needs two visible triangle meshes — show or open the opposing scan first

contact-details = Details
contact-details-hint = The numbers and the one colour per contact rule
contact-details-close = Hide details
settings-shortcuts-hint = Keyboard and mouse reference (F1)
