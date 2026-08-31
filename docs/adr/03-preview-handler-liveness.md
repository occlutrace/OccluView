# ADR 03: Keep Explorer independent of live-preview rendering

**Status:** Accepted
**Date:** 2026-08-31
**Baseline:** `f8780e4b967e074bbbdc2e57a9fcb3c10ac2e084`

## Context

The Windows Preview Pane can show an OccluView frame and then leave Explorer
unresponsive. The current handler calls the full parse, GPU upload, render,
readback and GDI-update path synchronously from `IPreviewHandler::DoPreview`.
After a first frame, `SetRect` moves the child window synchronously; `WM_SIZE`
runs the same path again.

Offscreen readback currently waits without a deadline for both the GPU poll and
the map callback. A panic guard cannot recover a thread waiting in a driver,
COM call, or mutex. Explorer normally hosts a Preview Handler in `Prevhost`,
but it waits synchronously for handler calls, so a blocked host is visible as a
blocked Explorer window.

The installed CLSID has an `AppID` value but its corresponding `AppID` key does
not set `DllSurrogate=Prevhost.exe`. It therefore does not request a private
surrogate for OccluView. The previous fallback-adapter change is retained as a
risk reduction, but it is not proof that no graphics backend or driver is
touched.

## Decision

1. **COM calls stay small.** `SetWindow`, `SetRect`, `DoPreview` and pointer
   messages create or update the child window, update state, and schedule work;
   they do not synchronously parse or render a scan. Rendering is coalesced by
   one private window message after the COM call returns. Painting invalidates
   normally and never uses a synchronous `RDW_UPDATENOW` round trip.

2. **The rendering path has a finite budget.** The offscreen readback API
   accepts an explicit deadline. Shell preview uses a two-second deadline for
   GPU completion and map delivery. A timeout becomes a structured render
   error, retires the shared device, and leaves a deterministic placeholder;
   it never waits indefinitely in a Preview Handler process. The renderer is
   not asked to kill a thread or a driver call.

3. **Each installed OccluView Preview Handler has its own host.** A stable,
   OccluView-owned AppID is registered together with
   `DllSurrogate=Prevhost.exe`; the preview CLSID references it. MSI and
   `DllRegisterServer` write the same registry topology, and uninstall removes
   only that owned AppID. A stalled preview can then affect at most its own
   surrogate, not another provider sharing the default host.

4. **Diagnostics are opt-in and privacy-safe.** A diagnostic MSI is an
   optimized `release-unwind` build with PDBs. When the diagnostic registry
   switch is enabled, the shell writes bounded JSONL phase records under the
   low-integrity user's local application data. Records include process and
   request IDs, extension, byte count, dimensions, elapsed time and error
   class; they never include a scan path or scan content. The package includes
   explicit administrator-run helpers for WER crash dumps and ProcDump hang
   captures. Neither dump collection nor preview disabling is enabled by a
   normal release install.

5. **A diagnostic package is not a release.** It receives a distinct package
   label and checksum manifest, is verified through the existing Windows
   lifecycle harness, and is copied locally for owner testing. It is not
   tagged, published, or added to a GitHub Release.

## Consequences

The initial preview may arrive one message-loop turn later, which is
intentional: Explorer's UI and COM call return before expensive work begins.
The Preview Pane can show a placeholder after a bounded failure; that is a
safe failure instead of a desktop-wide perceived hang. A real-machine hang
dump remains the evidence required to determine whether the blocked native
operation was adapter creation, submission/readback, or USER/GDI.

## Verification

Tests must prove that the COM and `WM_SIZE` paths schedule instead of render,
the readback deadline produces an error, stale scheduled messages cannot use a
destroyed child window, and MSI/self-registration create the private AppID
with its `DllSurrogate` value. The Windows smoke harness must wait for an
asynchronously produced frame, exercise resize and interaction, and report
the host PID for a targeted dump. Static tests and CI do not replace an
owner-machine Explorer reproduction.

## References

- [Microsoft: Preview Handlers and Shell Preview Host]
- [Microsoft: IPreviewHandler]
- [Microsoft: ProcDump]
- [Microsoft: Collecting User-Mode Dumps]

[Microsoft: Preview Handlers and Shell Preview Host]: https://learn.microsoft.com/en-us/windows/win32/shell/preview-handlers
[Microsoft: IPreviewHandler]: https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nn-shobjidl_core-ipreviewhandler
[Microsoft: ProcDump]: https://learn.microsoft.com/en-us/sysinternals/downloads/procdump
[Microsoft: Collecting User-Mode Dumps]: https://learn.microsoft.com/en-us/windows/win32/wer/collecting-user-mode-dumps
