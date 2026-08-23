# Using OccluView

Everything below is what the shipped build does. Nothing here is planned or
aspirational; each binding was read out of the code that implements it.

## Opening scans

Open a file from the toolbar, from *Recent*, or with **Ctrl+O**. Drag a file
onto the window to do the same thing.

Opening a second file while one is loaded **adds** it as another layer, which
is how you get an upper and a lower arch into one scene. Choosing *Open* from
the toolbar **replaces** the scene instead — and if the current session has
unsaved mesh edits, OccluView asks before destroying them.

Supported files are listed in the README. `.dcm` here means a dental HPS
container, not medical DICOM; DICOM files are recognised and refused rather
than half-read.

## Moving the camera

The viewport is orthographic, so a measurement on screen is a measurement.

| Action | Binding |
| --- | --- |
| Orbit | Right-drag |
| Pan | Middle-drag, or left+right drag together |
| Zoom | Wheel |
| Re-centre what you orbit around | Double-click, or middle-click, a point on the surface |
| Snap to an axis | Click a marker on the axis gizmo (bottom-right) |

Re-centring moves the orbit pivot to the point under the cursor and leaves the
zoom alone, so the scan does not jump. On empty space it does nothing, because
there is no surface point to centre on.

The scale bar in the corner is drawn from the camera, so it is correct at
every zoom level.

## Layers

Each opened scan is a layer. The Layers panel controls visibility, opacity,
tint, wireframe, vertex colours and texture per layer, and the right-click menu
adds the mesh operations below.

Three shortcuts live outside the panel, all on the middle button over a layer:

- **Ctrl+Middle-click** hides it.
- **Ctrl+Shift+Middle-click** restores the most recently hidden one.
- **Shift+Middle-click** toggles its translucency.

Tints come in two groups. *Model* shades are neighbours on one warm band, for
scans that should look like scans. *Overlay* colours are strong oppositions —
Cobalt against Tangerine first — for telling two scans apart where they
overlap, which is exactly where a warm band fails.

## Editing a mesh

Right-click a layer and choose *Edit mesh* to open a scene-wide edit session.

| Action | Binding |
| --- | --- |
| Select faces | Click, rectangle drag, or lasso |
| Select everything visible | **Ctrl+A** |
| Delete the selection | **Delete** or **Backspace** |
| Undo / redo | **Ctrl+Z** / **Ctrl+Y** (or **Ctrl+Shift+Z**) |
| Close the outline you are drawing | **Enter**, double-click, or click the first point |
| Abandon the outline you are drawing | **Esc** |

**Esc** cancels the outline in progress and nothing else: it does not leave the
tool, and with no outline on screen it keeps its ordinary meaning for whatever
is in front. The lasso and Object pick are turned off the same way they were
turned on, from the Mesh Editor's toolbar.

The operations that act on a selection — delete, crop to selection, cut the
selection to a new layer, separate connected components, close holes — are
buttons in the Mesh Editor panel, not menu items: they need a selection only
this panel can make. The layer's right-click menu carries the ones that act on
the whole layer — repair, invert normals, export, and the display switches. A
large edit blocks the window while it runs; that is known and measured, not a
hang.

When an operation cannot be undone, the status line says so instead of
promising Ctrl+Z.

## Sculpting

The Mesh Editor's sculpt tab paints on the surface.

| Action | Binding |
| --- | --- |
| Add / Remove brush | **1** |
| Smooth brush | **2** |
| Apply the brush | Left-drag on the surface |
| Invert the brush | Hold **Shift** |

Shift means "carve" for Add/Remove and "force it" for Smooth — Smooth converges,
so its Shift raises the strength to maximum and widens the footprint by 1.75x,
which is the lever that actually smooths harder. The cursor ring shows the
widened footprint while Shift is held.

Each stroke is one undo step.

## Measuring

The ruler measures along the surface, not through it. The thickness probe
reports the distance between the surface under the cursor and the far side of
the same scan.

**Esc** closes the active measuring tool and drops its overlays, including a
cut view the thickness probe opened.

## The cut view

Plant the cutting disc on the surface, then drag it to move the section. The
Section panel shows the slice, with its own wheel: plain wheel zooms the panel,
**Ctrl+wheel** resizes the disc.

**Esc** steps back one rung: it unplants a planted disc, and closes the cut
view when nothing is planted. **F** flips the section over — it keeps the half
that was being cut away — and only does anything while the disc is planted.

When the thickness probe is driving the cut, the disc follows it and stays
passive — the probe owns the pointer, so **Esc** closes both together.

## Aligning two scans

*Align Scans* registers a moving scan onto a fixed one and colours the result by
deviation.

Drag to place the scans roughly, constrain the drag to an axis or a plane from
the panel when a case needs it, mark regions to include or exclude with the
brush, then run the fit. The heatmap shows where the two surfaces agree.

The axis constraint belongs to the session: closing the tool clears it, so it
never carries into the next pair of scans.

## Headless: `occluview-cli`

Installed alongside the viewer, on `PATH`.

```
occluview-cli thumbnail <file> [-o out.png] [--size N]
occluview-cli convert   <file> -o output.{stl|ply|obj}
occluview-cli close-holes <file> -o out.stl [--limit-mm N]
occluview-cli info      <file> [file...]
occluview-cli --version
```

`thumbnail` is the same path the Windows shell extension and the Linux
thumbnailer use, so a correct PNG here means correct previews in the file
manager. Set `RUST_LOG=debug` to see why a file produced a placeholder instead
of a render.

## Windows Explorer

The MSI installs a thumbnail provider and a live Preview Pane handler. The
preview is interactive: right-drag orbits, the wheel zooms, **F** frames the
model and **W** toggles wireframe, and right-click offers the same view presets
as the app.

## When something goes wrong

If the viewer closes unexpectedly it writes a crash report before it goes:

- Windows: `%APPDATA%\OccluView\crashes\`
- Linux: `~/.local/state/OccluView/crashes/`

The report holds the version, the thread, the crash location and the last log
lines. It deliberately contains no scan paths. Attaching it to an issue is the
most useful thing you can do; the README's *What OccluView stores on this
machine* section lists everything else kept locally and how to clear it.
