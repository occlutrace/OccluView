# ADR 02: Staged platform modernization and compact interface redesign

**Status:** Accepted
**Date:** 2026-08-30
**Baseline:** `ddca41208a8956356a5375035d94d4bcef8f8581`
**Supersedes:** the compact Settings design and its implementation plan

## Context

The baseline uses Rust 1.86.0, edition 2021, egui/eframe 0.29.1, and wgpu
22.1.0. Its full locked workspace build and test suite pass. The clean
`cleanup/settings-redesign-20260830` branch remains the rollback reference for
the existing Settings behavior and icon renderer, but not the final visual
design.

The target stack is Rust 1.98.0, egui/eframe 0.36.1, and wgpu 30.0.1. egui
0.36.1 requires Rust 1.95 and uses wgpu 30, so the compiler moves first. The
upstream facts are recorded in the [Rust release index], [egui 0.36.1 workspace
manifest], [egui changelog], and [wgpu 30.0.1 release notes].

A disposable compile probe measured the migration: 113 renderer errors, then
76 application errors across 27 files. Ten production renderer files use wgpu
directly, the app has one direct GPU bridge, and 76 Rust files use egui or
eframe. Compatibility and visual redesign therefore require separate green
checkpoints.

[Rust release index]: https://blog.rust-lang.org/releases/latest/
[egui 0.36.1 workspace manifest]: https://github.com/emilk/egui/blob/0.36.1/Cargo.toml
[egui changelog]: https://github.com/emilk/egui/blob/0.36.1/CHANGELOG.md
[wgpu 30.0.1 release notes]: https://github.com/gfx-rs/wgpu/releases/tag/v30.0.1

## Decision

Work proceeds through serial, independently reviewed checkpoints. Each later
checkpoint starts from the preceding green commit. No push, tag, merge,
publication, release version bump, or release signing is authorized here.

Intermediate versions may be used only as disposable local diagnostics. Public
history records coherent engineering outcomes, not mechanical probes, tool
attribution, agent narration, or generated audit dumps.

### Stage 0: repair the packaging baseline

Port only the proven Windows Explorer cache-refresh correction:

- both deferred `--shell-refresh` WiX actions use `Impersonate="yes"` and
  `TerminalServerAware="yes"`;
- a source contract rejects either action running as LocalSystem;
- no version, lockfile, UI, or changelog changes are imported from the private
  branch.

Every Cargo invocation contributing to a candidate package must use `--locked`.
This includes Linux Debian, native MSI, GNU Windows cross-build, and the
embedded-key package paths in CI. The sole exception is cargo-cyclonedx 0.5.8,
whose `cargo cyclonedx` subcommand has no `--locked` option. Its equivalent
guard is, in order: an immediately preceding
`cargo metadata --locked --format-version 1`, SBOM generation, and
`git diff --exit-code -- Cargo.lock` before the SBOM is moved or copied into
package artifacts. Source-contract tests enforce the default rule and this
exact exception for both package workflows. No other exception is authorized;
a missing `--locked` or equivalent guard blocks this stage.

Local source checks do not prove Explorer behavior. MSI acceptance requires a
non-publishing lifecycle run using a known prior release as `-MsiPath` and the
candidate as `-UpgradeMsiPath`, followed by an installing-user Explorer check
for thumbnails and Preview without restarting Explorer. These Windows-only
checks remain explicitly unverified until suitable execution is available.

### Stage 1: move the compiler only

Pin Rust 1.98.0 in the toolchain file, workspace MSRV, every stable CI and
package-workflow toolchain entry, `docs/ARCHITECTURE.md`, and the old-version
comments in `scripts/run-fuzz.sh` and `scripts/gen-third-party.sh`. Pin
`clippy.toml`'s MSRV-aware lint behavior to Rust 1.98.

Keep edition 2021 and the complete dependency graph unchanged. Neither
`Cargo.lock` nor `fuzz/Cargo.lock` may change. No file under `crates/` changes in
the compiler-pin commit. If Rust 1.98 exposes a real source incompatibility, it
is fixed and reviewed in a separate preparatory commit.

### Stage 2: migrate egui, eframe, and wgpu without redesign

Target exactly egui 0.36.1, eframe 0.36.1, and wgpu 30.0.1. Only transitive
lockfile changes required by that coherent graph are accepted. Direct `rfd`,
`x11rb`, `windows`, `windows-core`, `resvg`, `image`, and `zip` dependencies
remain pinned. The eframe edge keeps `default-features = false` and enables
exactly `wgpu`, `default_fonts`, and `links`; `links` is required by the
existing Settings/About `ctx.open_url` actions because native URL opening is
opt-in in eframe 0.36. The accepted pollster graph is 0.3.0 through pinned
`rfd 0.14.1`, direct 0.4.0 in `occluview-app` and `occluview-thumbnail`, and
1.0.1 through `eframe 0.36.1`. Global duplicate elimination is not an
acceptance criterion.

Implementation order is serial:

1. resolve one egui, eframe, egui-wgpu, and wgpu graph;
2. adapt `occluview-render` while preserving rendered intent;
3. adapt eframe lifecycle, root `Ui`, bootstrap, and `live_viewport`;
4. adapt panels, dialogs, popups, input, theme drawing, and file drops;
5. remove compatibility-only scaffolding and run behavior tests.

Stage 2 uses the smallest current-API equivalent and deliberately preserves the
baseline presentation. Renderer contracts for colour space, readback,
shared-device rendering, ghost/cut pipelines, measured maps, device loss, and
texture destruction remain intact. The texture-destruction workaround is
removed only after a regression test first proves it obsolete.

The eframe 0.36 lifecycle is an ownership boundary, not a mechanical rename.
`App::logic` owns settings retries, status expiry, background load and sculpt
polling, open-request handling, update-channel polling, and interception of an
unsaved close. That interception must send `CancelClose` from `logic`, because
eframe can execute it without `ui` while the window is hidden. `App::ui` owns
dropped-file input, shortcuts, texture work, both same-frame render passes,
the root toolbar and central panel, GPU-error surfacing, and every visible
dialog. Manual update checks and downloads start after `logic`, so those UI
transitions schedule the first bounded repaint; recurring polling remains in
`logic`. The live viewport continues to wrap clones of eframe's existing
`Device` and `Queue` handles at the renderer's `Arc` boundary and never creates
a second GPU for the live path.

Stage 2 keeps Settings as a foreground `Area`, not a memory-backed `Popup`:
egui stores one active popup id per viewport, so making Settings a popup would
let its child `ComboBox` displace the parent. Recent files and tint palettes use
the current `Popup` API with explicit bottom-start alignment, no flip
alternatives, top-down justified layout, and close-only-the-owner semantics.
Window and floating-surface bounds use `content_rect`, preserving safe-area
insets rather than drawing under platform chrome.

Feature unification is not permission to expand the product's file-format
contract. `egui-winit 0.36.1` enables arboard's `image-data` feature; on the
Windows target this activates `image 0.25.9`'s BMP codec in the same workspace
graph used by HPS and glTF texture readers. Task 7 makes this contract
platform-independent by adding
`image = { workspace = true, features = ["bmp"] }` as a test-only dependency
in both `crates/occluview-formats/Cargo.toml` and
`crates/occluview-hps/Cargo.toml`. The deliberate dev-dependency feature edge
must make the BMP decoder available in each reader's own test build; it does
not change the production image dependency declaration.

The tests exercise the real glTF path in
`crates/occluview-formats/src/gltf/tests.rs` and the real HPS container path in
`crates/occluview-hps/src/texture_tests.rs`. Each module owns the exact tests
`embedded_png_and_jpeg_remain_accepted`,
`valid_bmp_is_rejected_by_whitelist_before_decode`, and
`malformed_bmp_magic_is_rejected_by_whitelist_before_decode`. Before the
production edit, the valid-BMP tests must run one test each and fail because
the current readers successfully decode the valid BMP while the decoder is
present. A zero-test filter or a missing BMP codec is not an acceptable RED.
The malformed fixture starts with BMP magic but is truncated; its final error
must be the same whitelist-specific rejection as the valid BMP, not a decoder
failure.

Both production readers must detect the embedded raster from bytes before
constructing a decoder. A typed helper returns `image::ImageFormat` only for
`ImageFormat::Png` or `ImageFormat::Jpeg`; every other detected format is
rejected, and the accepted value is passed explicitly to `ImageReader`.
Container MIME strings and filename extensions are not authoritative. TIFF
appears only as macOS-target lock metadata through arboard and is not a shipped
Windows/Linux acceptance consequence.

### Stage 3: redesign the application shell and Settings

After Stage 2 is green, rebuild the visible shell as a calm clinical
instrument: dense, neutral, precise, and subordinate to the model viewport.
This is a deliberate full redesign, not a compatibility side effect. Existing
workflows, commands, shortcuts, file formats, renderer behavior, persistence,
and update semantics remain product contracts.

#### Ownership and data flow

- `app_toolbar` owns toolbar composition; dialog code owns only dialogs and
  guards; Settings stays a focused module.
- Reusable chrome lives in focused UI modules as `ChromeBar`, `ToolbarGroup`,
  `ToolButton`, `FloatingPanelFrame`, `PopupSurface`, `ModalSurface`,
  `PanelHeader`, `SectionLabel`, `PropertyRow`, `ActionRow`, `DialogActions`,
  `InlineStatus`, `ViewportOverlayLayout`, and `IconTextAtom` equivalents.
- Rendering functions receive read-only view state and return one typed action.
  The Settings popup's open state lives only in egui `Popup` memory, never in a
  duplicate application boolean. The application owns file I/O, persistence,
  update requests, worker polling, and modal transitions. Its `InformationDialog`
  enum is the sole route for the mutually exclusive About and third-party
  licence surfaces.
- Persistence returns `Result`; a failed write remains dirty and retries rather
  than silently becoming clean.
- Startup and manual update checks share one typed state machine. A network
  failure is never presented as “current”.
- `eframe::App::logic` is reserved for nonvisual polling/state work and
  `eframe::App::ui` for drawing and interaction.

#### Toolbar and viewport

Preserve the command order: Open/recent, Add, Cut, Ruler, Thickness, Align,
Edit, Settings. Settings is a true toggle and remains active while its popup is
open; no control paints a fake close glyph.

The chrome bar is 36 points high with 28-point controls, 8-point outer padding,
and visible group separation. At widths of at least 900 points controls use
icon and text. From 680 through 899, Open, Add, and Settings retain text while
tool controls become icon-only. Below 680, file actions and Settings remain
visible and tools move into an overflow menu. Keyboard paths and tooltips remain
available in every mode.

The viewport keeps a 12-point safe area. Layers and its menu occupy the
upper-right anchor; Settings shares that edge only while open and the overlay
layout prevents collision. Mesh editing stays lower-left, alignment/exclusion
controls upper-left, and status/scale/readouts use the bottom corners. Floating
panels negotiate bounds and use internal scroll only when their content cannot
fit the viewport.

#### Settings and About

The product uses the single label “Settings”. It is a short command-adjacent
popover, never a second application window: 312 points wide, content-derived
height, and an 8-point viewport inset. The trigger and popover use egui's
memory-backed `Popup` with bottom-end alignment and close-on-outside-click.
This deliberately replaces the temporary foreground `Area`: Settings now has
no child popups, so the current primitive supplies one source of truth for the
toggle state, outside click, and Escape rather than hand-rolled dismissal
rules. Scrolling is allowed only when a genuinely compact viewport cannot
contain the content.

Settings exposes exactly the durable preferences already supported:

- fallback export format as one direct, three-option PLY/STL/OBJ segmented
  control, with the selected format in solid neutral ink;
- remember export directory as a single labelled toggle;
- check for updates on startup as a single labelled toggle;
- a compact manual Check now row with typed checking/current/available/failed
  status; and
- About as a separate transition.

Recent-file clearing stays in Open. Open resets the camera; Add preserves the
view. These are commands, not preferences. No theme selector, onboarding,
command palette, docking system, speculative preference, or duplicate control
is introduced. The panel has no decorative hero, product card, duplicated
section title, or Save/Cancel workflow: the existing retrying persistence path
is immediate, and a real persistence failure is one compact inline message.

About is a true `egui::Modal`, nominally 320 points wide with a 16-point
viewport margin and an 8–12% neutral scrim. It retains the logo, product name,
subtitle, version, links, third-party licenses, product license, and Close.
Focus is contained, background interaction is blocked, and Settings -> About
-> Third-party remains a typed, mutually exclusive transition.

#### Dialogs, menus, and visual system

Guard dialogs share a modal surface capped at 440 points. Save is primary,
Cancel is safe/default, destructive actions are separated and semantically
dangerous without becoming the visually loudest action. Error dialogs have
higher priority, remain usable with long content, and may resize within the
viewport.

Menus are nominally 244 points wide with 24–26-point rows and an 18-point icon
gutter. They support keyboard navigation, remain in bounds, explain disabled
items, and isolate destructive Remove actions.

Use a 4-point spacing grid: 4, 8, 12, 16, and 24. Controls are 28–30 points
high; type is approximately 14-point title, 12.5-point body/control, 11-point
metadata, and 18–19-point About product name. Controls use 4-point corners and
floating surfaces 8-point corners. Shadows belong only to floating surfaces.
Colour stays neutral with restrained semantic warning and danger accents.
Motion is functional, at most 120 ms, and never decorative.

Keep the curated 52-icon Lucide 0.544.0 snapshot, its license, and the resvg
0.48.1 renderer. Use current egui `Popup`, `Modal`, `Panel`, `CornerRadius`,
stroke/margin/shadow types, scroll input, and `Atom` composition where their
behavior matches these contracts.

#### UI proof matrix

Before visual implementation, extend the production-path immediate-mode
harness; do not maintain a duplicate test-only UI skeleton. The direct format
selector removes the child-ComboBox fixture. Tests cover the actual popup
toggle, direct format selection, toggles, update states, inline persistence
error, Settings -> About -> Third-party, outside click, Escape, focus, and
modal exclusion. Remove source-grep assertions for style or interaction once
equivalent behavioral coverage exists. Keep source contracts only for
architecture, packaging, or security properties that source inspection
genuinely proves.

The deterministic UI suite covers 1024x768, 800x600, and 500x384 viewports at
1x and 2x scale. It covers default, update checking/failed/available,
persistence failure, Settings -> About -> Third-party, every guard, error
modal, menu, and overlapping viewport-panel state. It exercises
toolbar triggering, selection and persistence, outside click, Escape, focus,
keyboard navigation, modal exclusion, responsive toolbar modes, and overlay
collision. Screenshot baselines live in a documented test-data location; CI
uses a fixed backend/font environment, a declared tolerance, and emits failure
diff artifacts. The exact local and CI commands are added with the harness.

### Stage 4: review remaining dependencies by risk cohort

Do not maximize version numbers. Each accepted cohort receives its own lockfile
review, tests, notice diff, and commit.

- Consider `image` 0.25.10 only with malformed/adversarial image and GLB,
  golden-thumbnail, allocation, and memory-bound tests.
- Consider rustls 0.23.43, rustls-webpki 0.103.15, and webpki-roots 1.0.9 only
  with signed-update, bad-certificate, bad-signature, and offline tests. Keep
  `ureq` 2.12.1 until a separate 3.4 API and manifest review.
- Defer `rfd` 0.17, `glam` 0.33, `thiserror` 2, `sha2` 0.11, `zip` prereleases,
  and `manifold-csg` changes to separately justified migrations.
- Keep cargo-about, cargo-cyclonedx, cargo-fuzz, cargo-deny action, and cargo-xwin
  policy unchanged until their generated artifacts or platform behavior can be
  reviewed independently.

Edition 2024 is a later language migration and is outside this ADR.

## Dependency and supply-chain policy

Every lockfile change is reviewed package by package. Deleted malicious crates
and versions identified by the Rust Security Response Team, including
`arrayref` 0.3.10, are rejected; the baseline uses safe 0.3.9. Root and excluded
fuzz graphs are checked separately. Because nested worktrees confuse the fuzz
workspace boundary, fuzz verification runs from a standalone checkout outside
`.worktrees/` when necessary.

[Rust Security Response Team notice]: https://blog.rust-lang.org/2026/08/20/supply-chain-attack-on-arrayref/

Each lock-changing stage runs cargo-deny, regenerates notices and reviews the
exact diff, reviews both lockfiles, and uses locked package commands. A
cargo-about upgrade is not coupled to this migration.

The resolved `epaint_default_fonts 0.36.1` manifest declares
`(MIT OR Apache-2.0) AND OFL-1.1 AND Ubuntu-font-1.0`. Task 8 replaces the
stale scoped `LicenseRef-UFL-1.0` exception with the current SPDX identifier,
updates the checked font clarification, and regenerates notices from the
locked graph. The generated Ubuntu Font Licence text and its
`epaint_default_fonts 0.36.1` attribution are verified, as is byte-stable
notice generation and `cargo deny check all`. The license remains scoped to
`epaint_default_fonts`; it is not added to the global allow-list merely to
silence a check.

## Verification gates

Every stage passes its relevant subset before the next starts. The final local
branch must pass all locally available gates:

1. exact Rust/Cargo version and dependency-tree checks;
2. `cargo fmt --all -- --check`;
3. locked workspace clippy with warnings denied;
4. locked workspace tests with no fail-fast;
5. shipped HPS/formats feature-set tests and clippy, plus each reader's
   BMP-enabled test graph and the six exact production-path PNG/JPEG/BMP
   contract tests named above;
6. rustdoc with warnings denied;
7. cargo-deny advisories, licenses, bans, and sources;
8. deterministic third-party notice generation and diff;
9. GNU Windows cross-check;
10. renderer golden, readback, error, and shared-device suites;
11. the Stage 3 UI interaction and screenshot matrix;
12. Linux release build and Xvfb runtime under the supported display paths;
13. locked Debian build, `check-deb.sh`, metadata, contents, AppStream, MIME,
    lintian, dynamic dependencies, and SHA-256;
14. disposable amd64 Debian/Ubuntu install, upgrade, launch, Open With,
    thumbnailer, and removal checks where the environment is available.

Linux runs the full renderer suite on Lavapipe. Windows CI/MSVC uses its stated
WARP pixel-suite exclusions; GNU cross-check is compile evidence only. A local
`.deb` without `OCCLUVIEW_HPS_EMBEDDED_KEY` is a technical candidate, not proof
of the shipped encrypted-HPS path. Windows MSI, live Explorer refresh,
hardware-driver behavior, and owner visual acceptance remain separate claims.

Before any future tag, record the final SHA, require ordinary CI green for that
SHA, and run `package-msi.yml` manually from the same SHA with
`release_dry_run: false` but without publication. Record run ID, SHA, artifact
names, and hashes. Release-equivalent packages additionally require the secret-
backed HPS path, contents/metadata/lint, SBOM, checksums, signature, and
attestation evidence.

## Versioning, history, and rollback

Any later release must keep workspace Cargo version, changelog, AppStream
release, WiX fallback/MSI resource version, `v<version>` tag, Debian control
version, and update-manifest version aligned. MSI versions remain numeric and
within `255.255.65535`.

Commits contain tests and their production change, stage explicit paths, and
exclude temporary probes and process metadata. Before publication, rollback is
the preceding green commit with its exact lockfiles; retain failed candidate
artifacts, hashes, and logs for diagnosis. After publication, recovery is a
higher-version repair carrying new MSI, Debian package, signed manifest,
checksums, SBOM, and attestation. Downgrade is not the normal recovery path.

## Acceptance

The branch is ready for owner evaluation when:

- all locally available stage and final gates are green;
- unavailable remote or owner-only gates are listed, never implied to pass;
- every existing workflow is reachable through the new compact shell;
- Settings is nominally 312 points wide, content-height at 1024x768, and fully
  bounded and operable at 500x384;
- the final graph contains the intended single GUI/GPU stack;
- the final local Debian technical candidate is validated and supplied with
  architecture, version, commit SHA, and SHA-256;
- the rollback branch and its package remain unchanged.
