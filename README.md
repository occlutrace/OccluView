<div align="center">
  <img src="assets/occluview-logo.png" width="96" height="96" alt="OccluView logo">

  <h1>OccluView</h1>

  <p><strong>Advanced Mesh Repair and Mesh Editing for digital dental CAD.</strong></p>

  <p>
    <a href="https://github.com/occlutrace/OccluView/releases/tag/v1.2.1"><img src="https://img.shields.io/badge/latest%20release-v1.2.1-1f6feb?style=for-the-badge&logo=github&logoColor=white" alt="Latest published release: v1.2.1"></a>
    <a href="https://github.com/occlutrace/OccluView/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/occlutrace/OccluView/ci.yml?branch=main&style=for-the-badge&label=build" alt="Build status"></a>
  </p>
</div>

OccluView is a native desktop dental CAD workspace for inspecting, aligning,
editing, repairing, and exporting scan meshes. It is designed for the work
that starts after a scan opens: compare two surfaces, find the deviation,
repair unsafe topology, edit and align 3D scans.

<hr>

## The workflow at a glance

<p align="center">
  <img src="assets/occluview-workflow.gif" width="820" alt="Animated OccluView workflow: overview, alignment heatmap, mesh editing, sculpting, and repair report">
</p>


## Download for Windows

Choose one file:

- [OccluView-Windows-Setup.msi](https://github.com/occlutrace/OccluView/releases/latest/download/OccluView-Windows-Setup.msi) — recommended; installs the viewer, Explorer previews, thumbnails, and file associations.
- [OccluView-Windows-Portable.zip](https://github.com/occlutrace/OccluView/releases/latest/download/OccluView-Windows-Portable.zip) — runs without installation; no Explorer integration.

## Quick Look

OccluView adds a live 3D preview to Windows Explorer. Select a scan and inspect
it immediately without opening the full viewer.

<p align="center">
  <img src="assets/explorer-preview.gif" width="640" alt="OccluView Quick Look showing a live 3D Explorer Preview Pane">
</p>

## Alignment and Heatmap

<p align="center">
  <img src="assets/alignment-heatmap.png" width="900" alt="OccluView Align Scans panel with a deviation heatmap over two dental scans">
</p>

Open both scans as layers, choose **A** (Align), and use the automatic workflow:

1. Confirm the moving and fixed scan in the panel.
2. Click `Best fit matching` to seat corresponding surfaces.
3. Read the colour map with the explicit millimetre legend and bounded range.

The heatmap is display-only evidence from the latest confirmed matching result.
Changing the pair, optimizer settings, exclusion markings, or returning to
Automatic clears it until a new matching result lands. Manual alignment remains
available when the automatic pair is not appropriate.

## Mesh Editing

<p align="center">
  <img src="assets/mesh-editing.png" width="900" alt="OccluView Mesh Editing panel with selection and mesh operations">
</p>

Mesh Editing keeps the common dental CAD operations in one bounded palette:

- lasso, object, surface, and through-mesh selection;
- select all, clear, and invert;
- delete, crop, cut, separate, and close safe holes;
- undo, redo, cancel, and an explicit Done commit.

<p align="center">
  <img src="assets/sculpting.png" width="900" alt="OccluView Sculpt panel with compact brush controls">
</p>

Sculpting uses the same editor session. Add/Remove and Smooth are separate
brush modes, while the size and force controls stay compact and readable in the
panel instead of consuming the whole viewport.

## Mesh Repair

<p align="center">
  <img src="assets/mesh-repair.png" width="900" alt="OccluView Mesh Repair report with concrete repair counts">
</p>

Mesh Repair runs on the selected layer and opens a bounded report card. It
removes duplicate and degenerate geometry, repairs unsafe topology, closes only
safe pinholes, and reports the non-zero changes together with remaining open
rims. A clean mesh receives an explicit “nothing to repair” result rather than
silence. `Copy details` preserves the full per-pass report for a case record.

## Files and results

Open STL, PLY, OBJ, GLB, and HPS dental containers. Export the finished result
as STL, PLY, or OBJ.

- `.hps` and `.dcm` are accepted as HPS dental containers; medical DICOM is not supported (a `DICM` signature is refused).

## Interface languages

OccluView follows the operating system language at startup. Settings can
override it with an explicit choice, which always wins over the OS setting.

Available: English, Русский, Deutsch, Español, Français, Italiano,
Português (Brasil). When the system language has no translation yet, the
interface renders English.

## Controls

Press **F1** — or open **Settings → Keyboard shortcuts** — for the complete keyboard and mouse reference.

- Open a scan with **Ctrl+O**. Opening another file adds a layer; toolbar Open
  replaces the scene.
- Toolbar tools: **C** Cut View, **M** Ruler, **T** Thickness, **A** Align, and
  **E** Mesh Editing.
- Orbit with right-drag; pan with middle-drag or LMB+RMB drag; zoom toward the
  pointer with the wheel; recenter on a surface with middle-click or
  double-click.
- In Mesh Editing, **Ctrl+A** selects all; **Delete** or **Backspace** removes;
  **Ctrl+Z**, **Ctrl+Y**, or **Ctrl+Shift+Z** undo and redo; **Enter** closes an
  outline; **Esc** cancels it.
- Mesh Repair is available from a layer's context menu and reports exactly what
  changed.
- In Sculpt, **1** chooses Add/Remove and **2** chooses Smooth. **Shift+wheel** changes Sculpt brush size; **Ctrl+wheel** changes Sculpt brush intensity.
  Holding **Shift** during a drag removes or strengthens the active brush mode.
- In Align, **Shift** erases an Align exclusion region; **Ctrl/Command+drag** rotates a scan in Align Manual mode; stationary **RMB click** undoes the last alignment point.
- Right-click a layer to read its **occlusal contacts**: the marks land on both
  scans, and the pointer reports the depth under the cursor.
- **Ctrl+Middle-click** hides a layer; **Ctrl+Shift+Middle-click** restores the
  last hidden layer; **Shift+Middle-click** toggles translucency.
- The bottom-right axis triad follows the camera and snaps to a labeled axis
  when an endpoint is clicked.

## Occlusal contacts

Right-click a scan and choose “Show contacts”. The scan is measured against
the scan it bites against — the nearest visible surface — and both arches are
painted where they meet. When the scene holds more than one candidate (an upper,
a lower, a pre-op, a wax-up), the details popover names the one being used and
lets you read against another layer instead; with one obvious candidate nothing
is asked. The panel carries two readings and one slider:

- “Contacts” marks only where the surfaces actually meet and colours each mark
  by how deep the bite is there, the way articulating paper leaves the rest of
  the tooth bare.
- “Approach” paints how close the other scan is everywhere, load included, for
  judging a jaw relationship rather than the contacts themselves.
- **Heavy at** moves the depth the ramp calls fully loaded. It recolours the
  measurement already in hand, so the boundary between a light contact and a
  heavy one is found by dragging rather than by re-measuring.
- **One colour per contact** flattens every patch to its deepest point. Leave it
  off to keep the distribution inside each mark. Changing it re-measures.

Point at either arch to read the value under the cursor in micrometres, with a
swatch of the exact colour the surface carries there; over a spot where nothing
was measured, the readout does not appear at all. The numbers under the slider —
contact area, patch count, deepest penetration — describe the scan the reading
was opened on, named first in the panel title. **Esc** closes the reading and
takes the marks off both scans.

Two scans further apart than half a millimetre are a legitimate reading with
nothing in it, not a failure: the panel says so and both arches stay bare.

## The cut view

Plant the disc on a surface, then drag it to position the section. Plain wheel
zooms the panel; **Ctrl+wheel** changes disc size. **F** flips the kept half while the disc is planted. **Esc** unplants the disc or closes Cut View.

## Windows Explorer

The MSI registers thumbnails and an interactive Preview Pane. In that pane,
right-drag orbits, wheel zooms, **F** frames the model, and **W** toggles wireframe.

## Command line

```text
occluview-cli thumbnail <file> [-o out.png] [--size N]
occluview-cli convert <file> -o output.{stl|ply|obj}
occluview-cli close-holes <file> -o out.stl [--limit-mm N]
occluview-cli info <file> [file...]
```

`thumbnail` uses the same rendering path as the Windows Preview Pane.

## Startup diagnostics and graphics requirements

OccluView needs a working desktop session and a graphics driver exposing one of
the wgpu backends supported by the platform. On Linux, the package declares the
X11/Wayland, Vulkan, and EGL loader packages; the actual Mesa or vendor GPU
driver is supplied by the operating system. Windows likewise needs a current
GPU driver for the platform's supported graphics backend.
The Windows MSI is x64; an unsupported OS architecture, a disabled graphics
adapter, or a driver that cannot create a surface can still prevent a window
from appearing.

If the installed viewer closes without a window, run:

```text
occluview --diagnostics
```

This performs a no-window adapter/device check and writes a report under the
OccluView state directory. It is a loader/device check, not proof that a GUI
surface can be created on the current desktop. Normal startup also performs
the device check before opening a window and tries available adapters in
preference order, so a bad discrete driver cannot hide a usable integrated
adapter. Startup failures write a unique report and return a non-zero process
status; reports are stored in the `crashes/` subdirectory, or in the system
temporary directory if the state directory is unavailable. Reports include the
last startup stages and recent log lines without including opened scan paths.
The metadata-only `startup-journal.log` follows the same state-directory then
temporary-directory fallback and records the last reached startup boundary
when a native driver failure happens before Rust can write a crash report.

The viewer renders with 4x multisampling when every graphics device it can start
on reports that it can create the multisampled targets, and with a single sample
otherwise. A machine that also exposes a device without that support starts
single-sampled; the `--diagnostics` report names each device's multisample
support next to the chosen count. If a driver rejects the multisampled window,
start once with `OCCLUVIEW_LIVE_MSAA=1` in the environment to force the
single-sample path; `OCCLUVIEW_LIVE_MSAA=4` forces it back on. A startup refusal
names the override.

Licensed under [Apache-2.0](LICENSE); distribution notices are in [NOTICE](NOTICE)
and [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md).
