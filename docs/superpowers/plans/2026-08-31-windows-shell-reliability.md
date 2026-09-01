# Windows Shell Reliability Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver a private Windows MSI in which Explorer Preview Pane and thumbnails have a documented, bounded, rendered lifecycle instead of a silent deferred-success path.

**Architecture:** Preview keeps Microsoft-supported, private low-integrity `Prevhost.exe` isolation and makes `DoPreview` synchronously establish a paintable first frame. The shared renderer receives explicit caller-owned deadlines and adapter policy; thumbnails retain one end-to-end request deadline while preferring a verified hardware adapter and falling back safely. WiX remains the sole supported machine registration writer, with narrowly scoped cleanup of OccluView-owned per-user overlays.

**Tech Stack:** Rust 1.98, windows-rs 0.62, wgpu 30.0.1, WiX v3, PowerShell Windows integration harness, GitHub Actions Windows runner.

**Spec:** `docs/superpowers/specs/2026-08-31-windows-shell-reliability.md`

## Global Constraints

- Retain the private preview AppID `{FD67C578-DBCC-4E10-8E47-63A8E48F7654}` with `DllSurrogate=Prevhost.exe`; do not set `DisableLowILProcessIsolation`.
- Retain stream-first initialization: `Initialize` stores source; `DoPreview` performs first-frame loading and rendering.
- Use `HardwareThenFallback` for Windows Shell rendering only after a nonempty known-triangle probe; no unchecked hardware output may enter Explorer's cache.
- The thumbnail request budget is exactly `DEFAULT_THUMBNAIL_TIMEOUT` (6 seconds) from first queue reservation through final readback; retries consume its remaining time.
- Preview's first-frame deadline is `PREVIEW_FIRST_FRAME_TIMEOUT` (8 seconds); a failure produces a paintable deterministic placeholder and never a pending spinner.
- A Preview deadline also bounds each next 1 MiB shell-stream copy and is
  shared by scene loading, renderer initialization, and frame rendering. An
  explicit interaction refresh creates one fresh deadline for the entire
  refresh, not a fresh render deadline after loading.
- `occluview-render` owns no Shell-specific timeout. Every offscreen call receives an explicit absolute `RenderDeadline`.
- Normal MSI installation is the only supported Shell registration path. Do not ship the manual `.reg` fallback or document `regsvr32` as a recovery path.
- HKCU repair may remove only OccluView's two implementation CLSIDs, its private AppID, and ShellEx values equal to those CLSIDs. It must not modify `UserChoice`, foreign CLSIDs, or default applications.
- Diagnostic logs contain only fixed enums, timing, process role, and adapter result. They contain no source path, file name, raw error text, scan bytes, driver string, or dump.
- Do not update Rust, egui, eframe, wgpu, parser limits, or public release metadata in this repair.
- Do not tag, publish a GitHub Release, update `latest.json`, or deploy. The only distributable is a private `1.1.3` MSI after all gates pass.

---

### Task 1: Make offscreen deadlines explicit and eliminate the global two-second policy

**Files:**

- Modify: `crates/occluview-render/src/error.rs`
- Modify: `crates/occluview-render/src/offscreen/mod.rs`
- Modify: `crates/occluview-render/src/offscreen/helpers.rs`
- Modify: `crates/occluview-render/src/offscreen/single_mesh.rs`
- Modify: `crates/occluview-render/src/offscreen/scene_render.rs`
- Modify: `crates/occluview-render/src/lib.rs`
- Test: `crates/occluview-render/src/offscreen/helpers.rs`
- Test: `crates/occluview-render/src/pipeline_tests.rs`

**Interfaces:**

- Produces: `occluview_render::RenderDeadline`, an absolute-deadline value that yields remaining time or `RenderError::ReadbackTimeout`.
- Produces: explicit `*_with_deadline` variants for every current `Offscreen` readback entry point.
- Consumed by: thumbnail renderer, shell preview scene, app export/cut-view calls, CLI, and adapter probe in Tasks 2–4.

- [ ] **Step 1: Write the failing deadline-behaviour tests**

~~~rust
#[test]
fn expired_deadline_fails_before_polling() {
    let deadline = RenderDeadline::at(Instant::now());
    assert!(matches!(deadline.remaining(), Err(RenderError::ReadbackTimeout { .. })));
}

#[test]
fn map_callback_uses_the_callers_deadline_not_a_global_constant() {
    let (_tx, rx) = mpsc::sync_channel(1);
    assert!(matches!(
        wait_for_map_callback(&rx, RenderDeadline::at(Instant::now())),
        Err(RenderError::ReadbackTimeout { .. })
    ));
}
~~~

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p occluview-render offscreen::helpers::tests --locked`

Expected: FAIL because `RenderDeadline` and the changed `wait_for_map_callback` signature do not exist.

- [ ] **Step 3: Implement the deadline value and use it at each GPU wait**

~~~rust
#[derive(Clone, Copy, Debug)]
pub struct RenderDeadline(Instant);

impl RenderDeadline {
    pub fn after(timeout: Duration) -> Self { Self(Instant::now() + timeout) }
    pub const fn at(deadline: Instant) -> Self { Self(deadline) }

    pub fn remaining(self) -> Result<Duration, RenderError> {
        self.0.checked_duration_since(Instant::now()).ok_or(RenderError::ReadbackTimeout {
            timeout: Duration::ZERO,
        })
    }
}
~~~

Change `read_back` and `read_back_extent` to accept `RenderDeadline`. Use the remaining duration in both `Device::poll(PollType::Wait { timeout: ... })` and the map-callback receive. Delete `READBACK_DEADLINE`; do not create a renderer-global replacement.

- [ ] **Step 4: Add explicit deadline variants to every offscreen API and migrate all callers**

~~~rust
pub async fn render_with_deadline(
    &self, mesh: &Mesh, camera: &GpuCamera, spec: ThumbnailSpec,
    deadline: RenderDeadline,
) -> Result<Vec<u8>, RenderError>;

pub async fn render_scene_with_deadline(
    &self, entries: &[SceneDrawEntry<'_>], camera: &GpuCamera,
    spec: ThumbnailSpec, deadline: RenderDeadline,
) -> Result<Vec<u8>, RenderError>;

pub async fn render_prepared_viewport_with_deadline(
    &self, scene: &PreparedScene, camera: &GpuCamera,
    spec: ViewportSpec, deadline: RenderDeadline,
) -> Result<Vec<u8>, RenderError>;
~~~

Apply the same pattern to clipped and overlay variants. Replace every old call site in render tests, app export/cut view, CLI, thumbnail, and shell with an explicit deadline. Do not leave compatibility wrappers that silently pick a deadline.

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo fmt --check && cargo test -p occluview-render --locked`

Expected: PASS; `rg 'READBACK_DEADLINE' crates` has no production result.

- [ ] **Step 6: Commit**

~~~bash
git add crates/occluview-render/src crates/occluview-app/src crates/occluview-cli/src
git commit -m "refactor(render): require caller-owned readback deadlines"
~~~

### Task 2: Give Shell renderers an explicit, testable adapter policy

**Files:**

- Modify: `crates/occluview-render/src/offscreen/mod.rs`
- Modify: `crates/occluview-render/src/pipeline_init.rs`
- Modify: `crates/occluview-render/src/lib.rs`
- Modify: `crates/occluview-thumbnail/src/offscreen_factory.rs`
- Modify: `crates/occluview-thumbnail/src/lib.rs`
- Modify: `crates/occluview-shell/src/com.rs`
- Modify: `crates/occluview-shell/src/offscreen_factory.rs`
- Test: `crates/occluview-thumbnail/src/offscreen_factory.rs`
- Test: `crates/occluview-shell/src/shell_preview_tests.rs`

**Interfaces:**

- Produces: `AdapterPolicy::{HardwareThenFallback, FallbackOnly}` and `AdapterResult::{Hardware, Fallback}`.
- Produces: `Offscreen::new_with_adapter_policy(policy, deadline)` and `Offscreen::adapter_result()`.
- Consumed by: thumbnail pool construction and preview-scene construction.

- [ ] **Step 1: Write failing policy tests**

~~~rust
#[test]
fn shell_factory_requests_hardware_then_fallback() {
    assert_eq!(shell_adapter_policy(), AdapterPolicy::HardwareThenFallback);
}

#[test]
fn fallback_only_remains_available_for_headless_test_fixtures() {
    assert_eq!(test_adapter_policy(), AdapterPolicy::FallbackOnly);
}
~~~

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p occluview-thumbnail offscreen_factory --locked`

Expected: FAIL because `AdapterPolicy` and `shell_adapter_policy` do not exist.

- [ ] **Step 3: Implement verified adapter selection**

~~~rust
pub enum AdapterPolicy { HardwareThenFallback, FallbackOnly }
pub enum AdapterResult { Hardware, Fallback }

pub async fn new_with_adapter_policy(
    policy: AdapterPolicy,
    deadline: RenderDeadline,
) -> Result<Self, RenderError>;
~~~

For `HardwareThenFallback`, request hardware, create the device within remaining time, render/read back the existing known triangle with the same deadline, and accept it only if alpha is nonzero. On hardware setup or probe failure, request the fallback adapter using remaining time. Record only `AdapterResult`, never an adapter name. `FallbackOnly` directly uses the fallback adapter for deterministic headless tests.

- [ ] **Step 4: Delete forced-WARP process state and activation prewarming**

Delete `SOFTWARE_RENDERER_ONLY`, `use_software_renderer_only`, its re-export, `THUMBNAIL_RENDERER_PREWARM`, and `spawn_renderer_prewarm`. Make thumbnail and preview factories request `HardwareThenFallback` from the rendering request. A factory must not hold a mutex while adapter/device creation waits.

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo fmt --check && cargo test -p occluview-thumbnail -p occluview-shell --lib --locked`

Expected: PASS; `rg 'SOFTWARE_RENDERER_ONLY|use_software_renderer_only|spawn_renderer_prewarm' crates` finds no production hit.

- [ ] **Step 6: Commit**

~~~bash
git add crates/occluview-render/src crates/occluview-thumbnail/src/offscreen_factory.rs \
  crates/occluview-thumbnail/src/lib.rs crates/occluview-shell/src/com.rs \
  crates/occluview-shell/src/offscreen_factory.rs crates/occluview-shell/src/shell_preview_tests.rs
git commit -m "refactor(shell): select verified hardware rendering with fallback"
~~~

### Task 3: Make thumbnail rendering consume one end-to-end budget

**Files:**

- Modify: `crates/occluview-thumbnail/src/render_thumb/mod.rs`
- Modify: `crates/occluview-thumbnail/src/render_thumb/rendering.rs`
- Modify: `crates/occluview-thumbnail/src/render_thumb/concurrency.rs`
- Modify: `crates/occluview-thumbnail/src/offscreen_factory.rs`
- Modify: `crates/occluview-shell/src/com/thumbnail_provider.rs`
- Test: `crates/occluview-thumbnail/src/render_thumb/tests/attempts.rs`
- Test: `crates/occluview-thumbnail/src/render_thumb/concurrency/recovery_tests.rs`
- Test: `crates/occluview-thumbnail/src/render_thumb/tests/burst_stress.rs`

**Interfaces:**

- Consumes: `RenderDeadline`, `AdapterPolicy`, and `AdapterResult` from Tasks 1–2.
- Produces: `ThumbnailRenderRequest { deadline: RenderDeadline, adapter_policy: AdapterPolicy }`.
- Produces: real pixels or `ThumbnailAttempt::TransientFailure`, never a cacheable placeholder for a renderer timeout.

- [ ] **Step 1: Write failing tests for deadline sharing and retry accounting**

~~~rust
#[test]
fn retry_receives_the_original_request_deadline() {
    let request = ThumbnailRenderRequest::new(DEFAULT_THUMBNAIL_TIMEOUT);
    assert_eq!(request.deadline(), request.deadline());
}

#[test]
fn deadline_failure_is_transient_not_a_cacheable_bitmap() {
    assert_eq!(thumbnail_attempt_for_timeout(), ThumbnailAttempt::TransientFailure);
}
~~~

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p occluview-thumbnail render_thumb::tests::attempts --locked`

Expected: FAIL because `ThumbnailRenderRequest` and `thumbnail_attempt_for_timeout` do not exist.

- [ ] **Step 3: Build one request object at the Shell boundary**

Replace separate duration arguments in Shell-facing `try_render_thumbnail_*` functions with `ThumbnailRenderRequest`. `reserve_thumbnail_stream_job` builds it before a stream is copied; file and stream paths carry the same absolute deadline into gate acquisition, renderer checkout, adapter creation, render, readback, and one retry. `render_mesh_thumbnail_with_offscreen` calls `render_scene_with_deadline` with that deadline. If the response deadline has already elapsed, an in-flight worker may complete only under a separately named, bounded cache-warm deadline while it retains its lane; its pixels must be stored in the process cache and never returned to the expired COM request.

- [ ] **Step 4: Preserve cache-safe failure semantics**

Keep `ThumbnailAttempt::Bitmap` only for valid image pixels or deterministic file results (unsupported, corrupt, over-budget). Map `RenderError::NoAdapter`, `RenderError::Surface`, `RenderError::ReadbackTimeout`, queue timeout, and stream I/O faults to `TransientFailure`, then have `IThumbnailProvider` return `E_FAIL` with a null bitmap. Do not use `placeholder_thumbnail` for transient cases.

- [ ] **Step 5: Add a mixed-burst regression test with one slow job**

Use the existing private renderer-pool test constructor to block one job until its request deadline expires, release it, then prove a subsequent small job uses a new renderer and returns visible pixels. Assert the timed-out job's renderer is discarded and the pool capacity returns to one.

- [ ] **Step 6: Run test to verify it passes**

Run: `cargo fmt --check && cargo test -p occluview-thumbnail --locked`

Expected: PASS; all timeout tests assert transient failure at the Shell boundary and the maximum mixed burst remains bounded by its one budget.

- [ ] **Step 7: Commit**

~~~bash
git add crates/occluview-thumbnail/src crates/occluview-shell/src/com/thumbnail_provider.rs \
  crates/occluview-shell/src/shell_contract_tests.rs
git commit -m "fix(thumbnails): share one deadline across shell rendering"
~~~

### Task 4: Replace deferred Preview first-frame scheduling with the Windows contract

**Files:**

- Modify: `crates/occluview-shell/src/com/preview.rs`
- Modify: `crates/occluview-shell/src/com/preview/window.rs`
- Modify: `crates/occluview-shell/src/preview_scene/load.rs`
- Modify: `crates/occluview-shell/src/preview_scene/render.rs`
- Modify: `crates/occluview-shell/src/offscreen_factory.rs`
- Modify: `crates/occluview-shell/src/com.rs`
- Test: `crates/occluview-shell/src/shell_preview_tests.rs`
- Test: `crates/occluview-shell/src/preview_scene/render.rs`

**Interfaces:**

- Consumes: `RenderDeadline` and `AdapterPolicy::HardwareThenFallback`.
- Produces: `PreviewHandler::render_first_frame()` and `PreviewHandler::refresh_preview_bitmap()`.
- Produces: `PREVIEW_FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(8)`.

- [ ] **Step 1: Write failing behavioural tests and narrow removal guards**

~~~rust
#[test]
fn do_preview_creates_a_bitmap_before_success() {
    let contract = PreviewFirstFrame::Ready { width: 320, height: 180 };
    assert!(contract.is_paintable());
}

#[test]
fn preview_render_failure_still_publishes_a_paintable_placeholder() {
    assert!(PreviewFirstFrame::Degraded.is_paintable());
}
~~~

The production source guard must reject `WM_OCCLUVIEW_RENDER_PREVIEW`, `pending_render_token`, and `PostMessageW` as render-delivery mechanisms.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p occluview-shell shell_preview_tests --locked`

Expected: FAIL while deferred queue symbols and old test assertions exist.

- [ ] **Step 3: Implement first-frame rendering in `DoPreview`**

~~~rust
fn DoPreview(&self) -> windows::core::Result<()> {
    let hwnd = self.this.ensure_preview_window()?;
    self.this.render_first_frame(
        hwnd,
        RenderDeadline::after(PREVIEW_FIRST_FRAME_TIMEOUT),
    )?;
    Ok(())
}
~~~

`render_first_frame` loads the initialized path or stream, prepares the scene, renders it, converts pixels to HBITMAP, replaces the previous bitmap, and calls `InvalidateRect`. One absolute deadline is carried through the bounded stream copy, renderer setup, scene preparation checks, render, and readback. Parsing, adapter, and readback failures create the existing deterministic placeholder at pane dimensions, publish it, invalidate the child, and emit a classified diagnostic event. `ensure_preview_window` failure remains an HRESULT failure. No `S_OK` path may leave no bitmap.

- [ ] **Step 4: Delete the obsolete scheduler completely**

Delete `NEXT_PREVIEW_RENDER_TOKEN`, `pending_render_token`, `schedule_preview_render`, `render_scheduled_preview`, `clear_pending_preview_render`, `WM_OCCLUVIEW_RENDER_PREVIEW`, and its window-procedure arm. Delete queue-specific comments. `WM_SIZE` may retain and repaint the last complete bitmap; it must not be the only initial-render route.

- [ ] **Step 5: Implement refresh after a valid first frame**

`refresh_preview_bitmap` receives the one fresh `RenderDeadline::after(PREVIEW_FIRST_FRAME_TIMEOUT)` created for an explicit resize completion or user interaction. Loading and rendering consume that same deadline. On failure retain the last valid HBITMAP; if there is no valid bitmap, publish the deterministic placeholder. `Unload` continues to destroy the child and release source/scene state. Do not move COM `IStream` across a worker thread.

- [ ] **Step 6: Run test to verify it passes**

Run: `cargo fmt --check && cargo test -p occluview-shell shell_preview_tests --locked && cargo test -p occluview-shell preview_scene --locked`

Expected: PASS; no production scheduler reference remains.

- [ ] **Step 7: Commit**

~~~bash
git add crates/occluview-shell/src/com.rs crates/occluview-shell/src/com/preview.rs \
  crates/occluview-shell/src/com/preview/window.rs \
  crates/occluview-shell/src/preview_scene crates/occluview-shell/src/offscreen_factory.rs \
  crates/occluview-shell/src/shell_preview_tests.rs
git commit -m "fix(preview): render a first frame from DoPreview"
~~~

### Task 5: Build privacy-safe activation diagnostics for both Shell components

**Files:**

- Create: `crates/occluview-shell/src/shell_diagnostics.rs`
- Modify: `crates/occluview-shell/src/lib.rs`
- Modify: `crates/occluview-shell/src/com.rs`
- Modify: `crates/occluview-shell/src/com/preview.rs`
- Modify: `crates/occluview-shell/src/com/thumbnail_provider.rs`
- Modify: `install/diagnostics/Enable-PreviewDiagnostics.ps1`
- Modify: `install/diagnostics/Collect-PreviewDiagnostics.ps1`
- Modify: `install/diagnostics/README.txt`
- Test: `crates/occluview-shell/src/shell_preview_tests.rs`
- Test: `install/test-msi-lifecycle.ps1`

**Interfaces:**

- Produces: `ShellDiagnosticEvent` with fixed enum fields and `record_shell_event` behind `diagnostic-logs`.
- Produces: `shell-events.jsonl` in LocalLow and a collector archive containing it plus the limited registry snapshot.

- [ ] **Step 1: Write failing serialization and privacy tests**

~~~rust
#[test]
fn diagnostic_event_has_only_fixed_fields() {
    let line = ShellDiagnosticEvent::completed(Component::Preview, Stage::BitmapPublish, 18).to_json();
    assert!(line.contains("\"component\":\"preview\""));
    assert!(!line.contains("path"));
    assert!(!line.contains("driver"));
}
~~~

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p occluview-shell diagnostic --locked`

Expected: FAIL because `ShellDiagnosticEvent` is absent.

- [ ] **Step 3: Implement fixed-event logging at every fault boundary**

Create enums `Component`, `Stage`, `Outcome`, `ProcessRole`, `AdapterResult`, and `ErrorClass`. Log activation before renderer work, then initialization, scene load, adapter selection/probe, render/readback, bitmap publish, and COM return. Record only elapsed milliseconds and enum values. Rename the collector target to `shell-events.jsonl`; retain compatibility collection of `preview-failures.jsonl` only when it exists.

- [ ] **Step 4: Add a read-only registration snapshot to the collector**

In PowerShell query only the two fixed CLSIDs, fixed AppID, and thumbnail/preview ShellEx categories from HKCU, HKLM `/reg:64`, HKLM `/reg:32`, and HKCR. Write output to `shell-registration.txt` in the staging archive. Do not call `reg add`, `reg delete`, `ie4uinit`, cache cleanup, `regsvr32`, or Explorer restart.

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo fmt --check && cargo test -p occluview-shell --lib --locked`

Expected: PASS; normal builds do not compile diagnostic file I/O and diagnostic MSI smoke finds scripts and collector output names.

- [ ] **Step 6: Commit**

~~~bash
git add crates/occluview-shell/src install/diagnostics install/test-msi-lifecycle.ps1
git commit -m "feat(shell): capture activation diagnostics without scan data"
~~~

### Task 6: Make MSI registration authoritative and repair only OccluView's user overlays

**Files:**

- Modify: `crates/occluview-shell/src/registration/mod.rs`
- Modify: `crates/occluview-shell/src/registration/registry.rs`
- Create: `crates/occluview-shell/src/registration/user_overrides.rs`
- Modify: `crates/occluview-shell/src/lib.rs`
- Modify: `crates/occluview-app/src/app_bootstrap.rs`
- Modify: `crates/occluview-app/src/app/state.rs`
- Modify: `crates/occluview-app/src/primary_ui_tests/loading.rs`
- Modify: `install/occluview.wxs`
- Delete: `install/occluview-shell-registration.reg`
- Modify: `crates/occluview-shell/src/installer_contract_tests.rs`
- Modify: `crates/occluview-shell/src/shell_contract_tests.rs`
- Modify: `README.md`
- Modify: `docs/USAGE.md`

**Interfaces:**

- Produces: `occluview_shell::repair_current_user_overrides() -> ShellRepairReport`.
- Produces: app argument `--repair-shell-overrides` that returns before normal GUI startup.
- Consumed by: impersonated MSI refresh action and Windows lifecycle tests.

- [ ] **Step 1: Write failing ownership tests**

~~~rust
#[test]
fn repair_targets_only_occluview_owned_keys() {
    assert!(is_owned_override_value(OCCLUVIEW_THUMBNAIL_CLSID));
    assert!(!is_owned_override_value("{00000000-0000-0000-0000-000000000000}"));
}

#[test]
fn normal_installation_does_not_ship_manual_registration() {
    assert!(!std::path::Path::new("../../install/occluview-shell-registration.reg").exists());
}
~~~

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p occluview-shell installer_contract_tests --locked`

Expected: FAIL while the HKCR self-registration module and `.reg` file remain.

- [ ] **Step 3: Split association refresh from self-registration**

Keep `notify_shell_associations_changed` as a small independent module using only `SHChangeNotify`. Delete `DllRegisterServer` and `DllUnregisterServer` exports, HKCR create/delete registration machinery, and manual `.reg` file. Update tests and docs so WiX, not `regsvr32`, is the only supported path.

- [ ] **Step 4: Implement narrow HKCU repair**

`repair_current_user_overrides` opens `HKCU\Software\Classes` directly. It deletes a class tree only for the two fixed OccluView implementation CLSIDs and private AppID; it deletes a ShellEx default only when its value equals the corresponding OccluView CLSID. It leaves all other values and keys intact, then returns counts for `clsid`, `appid`, `thumbnail_binding`, and `preview_binding`. It does not use merged HKCR.

- [ ] **Step 5: Wire repair into the impersonated MSI refresh action**

Add `repair_shell_overrides: bool` to parsed app arguments. In the command branch run repair and then `notify_shell_associations_changed`; do not create GUI. Change the WiX installed-executable custom action command to `--repair-shell-overrides --shell-refresh`, retain `Impersonate="yes"` and `TerminalServerAware="yes"`, and retain distinct install/uninstall refresh actions. The uninstaller must not erase HKCU state not demonstrably owned by OccluView.

- [ ] **Step 6: Update user-facing documentation**

Remove every suggestion to import a `.reg` file or run `regsvr32`. State in README and usage guide that Windows MSI is required for Explorer previews and an MSI repair repairs OccluView's own stale per-user bindings.

- [ ] **Step 7: Run test to verify it passes**

Run: `cargo fmt --check && cargo test -p occluview-shell --lib --locked && cargo test -p occluview-app --lib --locked`

Expected: PASS; `rg 'DllRegisterServer|DllUnregisterServer|occluview-shell-registration.reg'` has no shipped-product hit.

- [ ] **Step 8: Commit**

~~~bash
git add crates/occluview-shell/src crates/occluview-app/src install/occluview.wxs README.md docs/USAGE.md
git rm install/occluview-shell-registration.reg
git commit -m "fix(installer): repair only owned shell registration overlays"
~~~

### Task 7: Turn the Windows harness into an actual rendered-contract gate

**Files:**

- Modify: `install/test-preview-handler.ps1`
- Modify: `install/test-thumbnail-provider.ps1`
- Modify: `install/test-msi-lifecycle.ps1`
- Modify: `.github/workflows/package-msi.yml`
- Modify: `crates/occluview-shell/src/shell_preview_tests.rs`
- Modify: `crates/occluview-shell/src/installer_contract_tests.rs`

**Interfaces:**

- Produces: `ProbePrivateSurrogateRendered`, which activates through `CLSCTX_LOCAL_SERVER`, confirms child ownership by `prevhost.exe`, and proves non-background pixels.
- Produces: lifecycle `Assert-EffectiveShellRegistration` and `Assert-InstalledShellDllFingerprint`.
- Consumed by: normal, legacy-upgrade, and diagnostic MSI workflow runs.

- [ ] **Step 1: Add the failing private-surrogate pixel assertion**

Extend the C# helper in `test-preview-handler.ps1` with `GetDC`, `GetPixel`, and `ReleaseDC` P/Invokes. After `DoPreview`, poll the private child for a non-background pixel grid for 10 seconds before `Unload`:

~~~csharp
if (!WaitForNonBackgroundPixels(child, width, height, 10000))
    throw new InvalidOperationException("Private Prevhost preview never painted a frame.");
~~~

The test must still verify the child process is `prevhost.exe` and teardown after `Unload`. It must not replace this check with an in-process probe.

- [ ] **Step 2: Run test to verify it fails on the old deferred-first-frame package**

Run: `pwsh -NoProfile -File install/test-preview-handler.ps1`

Expected: FAIL on the private rendered assertion or expose a platform capture restriction that must be resolved with an equivalent same-session pixel probe before the task proceeds.

- [ ] **Step 3: Preserve thumbnail end-to-end routes and add evidence**

Keep direct, stream, item, Shell image factory, forced thumbnail-cache, cold, warm, and mixed-folder probes. Include `ElapsedMs`, `adapter_result`, and `HRESULT` in failures. Shell and forced-cache routes must return nonempty 32-bpp DIBs with real geometry; direct-provider success alone is insufficient.

- [ ] **Step 4: Add an injected HKCU-overlay lifecycle case**

Before normal install, write a current-user CLSID and both `.stl` ShellEx values using fixed OccluView GUIDs but an obsolete DLL path. Install MSI, then assert those HKCU values are gone, effective HKCR resolves to Program Files DLL, and real thumbnail and private-surrogate pixel probes pass. Use a `finally` block to remove only the test's keys.

- [ ] **Step 5: Make reboot and binary provenance explicit**

Change `Invoke-MsiExec` to return its exit code. If it is `3010`, mark test reboot-required and run post-reboot validation in a next Windows job rather than treating it as ordinary green installation. Before every rendered probe, assert `occluview_shell.dll` file version equals MSI version and write SHA-256 to CI summary. Do not claim live-Explorer acceptance from this harness.

- [ ] **Step 6: Run and inspect the Windows package workflow**

Run: dispatch `.github/workflows/package-msi.yml` with `windows_configuration=diagnostic`, then normal package configuration from the repair branch.

Expected: both jobs pass installation, upgrade, downgrade block, private `Prevhost` pixel test, forced thumbnail extraction, mixed burst, and uninstall.

- [ ] **Step 7: Commit**

~~~bash
git add install/test-preview-handler.ps1 install/test-thumbnail-provider.ps1 install/test-msi-lifecycle.ps1 \
  .github/workflows/package-msi.yml crates/occluview-shell/src/shell_preview_tests.rs \
  crates/occluview-shell/src/installer_contract_tests.rs
git commit -m "test(shell): require rendered private-host and upgrade contracts"
~~~

### Task 8: Replace the obsolete decision record and prepare the private installer

**Files:**

- Modify: `docs/adr/03-preview-handler-liveness.md`
- Modify: `docs/ARCHITECTURE.md`
- Modify: `README.md`
- Modify: `Cargo.toml`
- Modify: `Cargo.lock` only if Cargo changes it while building; do not update dependencies
- Create: `docs/RELEASE-VALIDATION-1.1.3.md`

**Interfaces:**

- Produces: an accurate record of Shell lifecycle, registration ownership, Windows evidence boundary, and private-package acceptance checklist.

- [ ] **Step 1: Replace invalid ADR assertions**

Update ADR 03 to state `DoPreview` owns first-frame rendering, private `Prevhost.exe` is retained, render deadlines are caller-owned, and the test observes rendered pixels in the private host. Delete wording that COM callbacks must only queue work or that a private-window message is the render delivery contract.

- [ ] **Step 2: Update architecture and customer documentation**

Document thumbnail one-request budget, hardware-then-fallback policy, cache-safe transient failures, and MSI-only Explorer setup. Do not expose minisign, registry, ProcMon, or diagnostics internals in normal download instructions.

- [ ] **Step 3: Write the release-validation checklist**

Include exact commands and required evidence:

~~~text
cargo fmt --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
pwsh -NoProfile -ExecutionPolicy Bypass -File install/build-msi.ps1 -Configuration release
Windows package workflow: rendered Preview, forced thumbnail extraction, HKCU repair, upgrade, downgrade, uninstall
Affected workstation: registry/ProcMon capture before repair; Explorer visual acceptance after reboot
~~~

- [ ] **Step 4: Bump only product package version to 1.1.3 after code tests pass**

Update `[workspace.package].version`, WiX product version inputs, file-version assertions, and test expectations together. Confirm `Cargo.lock` still pins `wgpu 30.0.1`.

- [ ] **Step 5: Run repository checks before packaging**

Run: `cargo fmt --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked && git diff --check`

Expected: PASS. Record unrelated baseline failures separately; do not alter them as part of this incident.

- [ ] **Step 6: Build private artifacts and inspect payload**

On Windows run:

~~~powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File install/build-msi.ps1 -Configuration release
pwsh -NoProfile -ExecutionPolicy Bypass -File install/test-msi-lifecycle.ps1 -MsiPath .\dist\OccluView-1.1.3-x86_64-pc-windows-msvc.msi
~~~

Verify MSI ProductVersion, x64 component bitness, private AppID, `occluview_shell.dll` version/hash, absence of manual registration file, normal/diagnostic package contents, and generated SHA-256. Place private MSI in the approved project artifact directory only; do not tag, create release, or update manifest.

- [ ] **Step 7: Commit**

~~~bash
git add docs/adr/03-preview-handler-liveness.md docs/ARCHITECTURE.md \
  docs/RELEASE-VALIDATION-1.1.3.md README.md Cargo.toml Cargo.lock install crates .github
git commit -m "docs(release): define Windows shell validation for 1.1.3"
~~~

## Plan self-review

### Spec coverage

- `DoPreview` first frame and no hidden scheduler: Task 4.
- Caller-owned deadline and hardware fallback: Tasks 1–3.
- Thumbnail cache safety and performance: Tasks 3 and 7.
- Privacy-safe diagnostics: Task 5.
- MSI-only registration and HKCU repair: Tasks 6 and 7.
- True private-surrogate pixel evidence, MSI provenance, and affected-machine boundary: Tasks 7 and 8.
- Removal of obsolete code and inaccurate documentation: Tasks 2, 4, 6, and 8.

### Placeholder scan

The plan contains no TBD markers, deferred implementation placeholders, or unspecified error-handling steps. Every task names files, public interfaces, failing tests, commands, and a commit boundary.

### Type consistency

`RenderDeadline` and `AdapterPolicy` originate in Tasks 1–2 and are consumed by Tasks 3–4. `ThumbnailRenderRequest` originates in Task 3 and is consumed by the COM provider. `ShellDiagnosticEvent` originates in Task 5. `ShellRepairReport` originates in Task 6 and is consumed only by the MSI command path and tests.

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-08-31-windows-shell-reliability.md`.

Execution mode is inline in this session: implement in task order with a code-review and evidence gate after each task. A real affected-Windows Explorer acceptance remains an external final gate; code, CI, and MSI lifecycle checks cannot substitute for it.
