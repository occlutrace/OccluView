# Windows Shell Reliability Design

**Status:** Approved for implementation on 2026-08-31

## Goal

Make OccluView's Windows Explorer thumbnails and Preview Pane reliable on a
real workstation: a supported file must produce a real image or a visible,
bounded failure state. Neither path may leave Explorer waiting forever,
silently return success without pixels, or rely on stale per-user registry
state.

## Evidence and problem statement

The 1.1.2 package has three separate defects. They must not be collapsed into
one speculative fix.

1. `IPreviewHandler::DoPreview` creates a child window, posts
   `WM_OCCLUVIEW_RENDER_PREVIEW`, and returns success. The render occurs only
   if that private message is later dispatched. In the actual private
   `Prevhost.exe` path the test currently proves only window creation and
   teardown; all pixel assertions run in-process and manually pump messages.
   A missed dispatch leaves `pending_render_token` set indefinitely and matches
   the observed perpetual spinner.
2. `occluview-render` applies one hard two-second GPU-readback limit to every
   offscreen caller. `occluview-thumbnail` has a six-second request budget,
   but its first WARP readback can fail at the unrelated two-second limit and
   becomes `E_FAIL` at the COM boundary.
3. The MSI correctly registers 64-bit classes under HKLM, but historical
   `DllRegisterServer` and the manual `.reg` file write through HKCR. A stale
   `HKCU\\Software\\Classes` entry with OccluView's own CLSID masks the MSI
   registration in Explorer's effective merged view.

The exact Registry-table comparison proves that the private preview AppID was
the only handler-registration difference between the known package lines; the
thumbnail CLSID and file-association rows did not change. The private AppID is
therefore retained, not rolled back.

## Authoritative platform contract

- A preview handler is an in-process COM server hosted out of process by
  `Prevhost.exe`; low-integrity hosting is the preferred and recommended
  Windows model. OccluView continues to use a private AppID with
  `DllSurrogate=Prevhost.exe`, so one bad preview does not share a host with
  unrelated preview providers.
- `IInitializeWithStream` remains the primary initialization path. It stores
  the stream but does not load it. `DoPreview` creates the child window, loads
  the source, begins rendering, and owns painting the supplied rectangle.
- `IThumbnailProvider` is stream-initialized on the normal out-of-process
  Shell path and must return a valid 32-bpp DIB with `S_OK`, or a real failure
  HRESULT. A transient renderer fault must never be turned into a cacheable
  placeholder bitmap.
- `HKEY_CLASSES_ROOT` is a merged HKCU/HKLM view. The installer owns the
  machine registration in HKLM; an upgrade may delete only HKCU values or
  classes identified by OccluView's fixed CLSIDs.

Sources: [Microsoft: Building Preview Handlers](https://learn.microsoft.com/en-us/windows/win32/shell/building-preview-handlers), [Microsoft: Preview Handlers and Shell Preview Host](https://learn.microsoft.com/en-us/windows/win32/shell/preview-handlers), [Microsoft: IPreviewHandler::DoPreview](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-ipreviewhandler-dopreview), [Microsoft: IThumbnailProvider](https://learn.microsoft.com/en-us/windows/win32/api/thumbcache/nn-thumbcache-ithumbnailprovider), [Microsoft: HKEY_CLASSES_ROOT merged view](https://learn.microsoft.com/en-us/windows/win32/sysinfo/merged-view-of-hkey-classes-root).

## Architecture

### 1. Shell callbacks are contracts, not an implicit scheduler

`DoPreview` becomes the one owner of first-frame work. It will:

1. validate its initialized source and create the child window after
   `SetWindow`;
2. load the preview scene and create/prepare the renderer;
3. render the first frame under a preview-specific deadline;
4. install an HBITMAP and invalidate the child only after pixels exist; or
5. install a deterministic, visibly degraded placeholder if source loading or
   rendering fails inside that deadline.

It returns `S_OK` only after the child has paintable pixels. It returns a
documented HRESULT for invalid initialization, corrupt source, inaccessible
source, or failed window creation. It never reports successful work that has
only been queued.

The private `WM_OCCLUVIEW_RENDER_PREVIEW` queue,
`pending_render_token`, and renderer work in the window procedure are removed.
Resize and interaction keep the last valid bitmap while a new bounded render
is performed through an explicit helper. No window procedure may be the sole
delivery mechanism for an initial image.

Preview retains the current private AppID and low-integrity `Prevhost.exe`.
`DisableLowILProcessIsolation` and a custom COM local server are explicitly
out of scope.

### 2. Rendering exposes a caller-owned deadline and adapter policy

`occluview-render` stops owning a hidden, universal two-second deadline. Its
offscreen entry points accept an explicit `RenderDeadline` that is created by
the caller from one absolute `Instant`. Rendering, GPU polling, map callback,
and retry all consume the same deadline; none receives a fresh nested timeout.
The one deliberate exception is an already-detached thumbnail cache warmer:
it has its own explicit six-second worker deadline, retains a bounded worker
slot, and is prohibited from returning a result to the expired Shell call.

The renderer exposes `AdapterPolicy`:

```rust
pub enum AdapterPolicy {
    HardwareThenFallback,
    FallbackOnly,
}

pub struct RenderDeadline(Instant);
```

`HardwareThenFallback` accepts a hardware adapter only after the existing
known-triangle draw/readback probe produces nonempty pixels. It then falls back
to WARP when hardware acquisition, the probe, or a later device fails. The
policy result is classified as `hardware` or `fallback`; raw adapter names,
driver strings, source paths, and source bytes are never written to product
diagnostics.

The app and CLI receive explicit policies as part of the API migration, so no
consumer accidentally inherits a Shell deadline. This P0 repair does not
upgrade wgpu: the workspace and lockfile already pin the current selected
`wgpu 30.0.1`; dependency migration is a separate audited change.

### 3. Thumbnail performance without cache poisoning

The thumbnail request's existing six-second end-to-end budget remains the
outer policy. It is created once when a request enters the Shell path and is
passed through queue acquisition, stream copy, mesh loading, renderer
creation, hardware probe, frame render, and readback. The two-second global
cap is deleted.

Normal Shell thumbnails use `HardwareThenFallback`; Windows normally isolates
the provider out of process. The existing renderer-pool retry discards an
unhealthy device, but every retry that can answer the active Shell request
consumes its remaining absolute deadline. A response-timeout continuation may
finish only as a separately bounded cache warmer; it remains inside its worker
lane and cannot answer the original COM call. The global
`use_software_renderer_only` flag and class-activation prewarm side effect are
deleted. A returned bitmap is a real render or a deterministic file verdict
only. Timeout, adapter fault, queue saturation, and stream I/O fault return a
failure HRESULT so Explorer retries instead of caching a false thumbnail.

### 4. Diagnostics designed for support, not for hope

The diagnostic MSI adds structured JSONL events at every boundary before a
potentially slow operation:

```text
schema_version, component, stage, outcome, elapsed_ms, process_role,
adapter_policy, adapter_result, error_class
```

Components are `preview` and `thumbnail`; stages include `com_activate`,
`initialize`, `do_preview`, `scene_load`, `adapter_create`, `adapter_probe`,
`frame_render`, `readback`, `bitmap_publish`, and `com_return`. Outcomes and
error classes are fixed enums. The logger records no patient file path, file
name, mesh bytes, raw driver error, registry values unrelated to OccluView, or
crash dump.

The collector includes this JSONL plus a read-only snapshot of only the
OccluView CLSIDs, AppID, and ShellEx values from HKCU, HKLM-64, HKLM-32, and
effective HKCR. It does not clear Explorer's thumbnail cache or repair keys.

### 5. Installer registration has one source of truth

WiX remains the only supported writer of machine shell registration. The
manual `.reg` fallback is removed from the package and documentation.
`DllRegisterServer` and `DllUnregisterServer` are removed as a supported
installation path; the DLL remains an ordinary COM in-process server loaded by
its WiX registration.

The MSI's impersonated user-context refresh action calls a narrow repair
command before `SHChangeNotify`. That command removes only per-user values or
class trees using OccluView's two fixed implementation CLSIDs, its fixed
private preview AppID, and ShellEx category values equal to those CLSIDs. It
does not modify `UserChoice`, a foreign CLSID, default application choice, or
any key whose value does not identify OccluView. It records counts by key kind
in the diagnostic mode only.

## Required acceptance evidence

### Code and test gates

- Rust unit tests prove that every render path receives a caller-owned
  deadline, no global `READBACK_DEADLINE` exists, retries share one deadline,
  and a fallback is used only after failed hardware acquisition or a failed
  nonempty-pixel probe.
- Preview tests prove that `DoPreview` creates a window and produces an HBITMAP
  before returning `S_OK`; no `WM_APP` render protocol or pending token remains.
- Windows integration uses `CLSCTX_LOCAL_SERVER` for the private
  `Prevhost.exe` handler and captures non-background pixels from its child
  window. In-process tests remain supplemental tests for input and geometry,
  not proof of the private host path.
- Windows thumbnail integration invokes the actual Shell image factory and a
  forced `IThumbnailCache` extraction, validates an ARGB DIB with nonempty
  geometry, records cold and warm duration, and runs a mixed-folder burst.
- MSI lifecycle starts from a deliberately injected stale HKCU OccluView
  overlay, installs/upgrades, proves the overlay has been removed, verifies
  effective HKCR points to Program Files, checks the installed DLL's file
  version and SHA-256 after any accepted `3010`, then repeats both rendered
  contracts.

### Human Windows acceptance

CI and a diagnostic package are necessary but not proof of the affected
workstation. Before clearing cache or re-registering anything on that machine,
capture the registry snapshot, event logs, and ProcMon trace. After the new
MSI is installed and the workstation is restarted when MSI requests it,
confirm all of the following in Explorer:

1. selecting a normal STL produces a visible Preview Pane frame;
2. a folder of STL, OBJ, PLY, GLB, and HPS files gets thumbnails without
   blocking Explorer;
3. opening, resizing, and switching files does not stall Explorer or leave a
   perpetual spinner; and
4. the diagnostic bundle reports a completed `bitmap_publish` for both
   components when diagnostic mode is used.

## Non-goals

- No public release, tag, GitHub Release, or update manifest publication.
- No deletion of the Windows thumbnail cache as a substitute for a fix.
- No weakening of parser bounds, low-integrity preview isolation, or COM panic
  guards.
- No broad cleanup of third-party registry keys or user default-app choices.
- No unrelated egui, Rust, or wgpu version update in this incident repair.

## Superseded design

This design supersedes the deferred-message first-frame decision in
`docs/adr/03-preview-handler-liveness.md`. The retained parts of that ADR are
private `Prevhost.exe` isolation, panic containment, stream-first loading, and
privacy-safe diagnostics. The queue-as-success contract, global readback
deadline, and `DllRegisterServer`-based support path are removed.
