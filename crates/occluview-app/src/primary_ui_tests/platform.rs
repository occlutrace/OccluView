use super::*;

#[test]
fn windows_app_reports_startup_and_panic_failures() {
    let source = app_bootstrap_source();
    let manifest = app_manifest_source();

    assert!(
        source.contains("install_panic_hook();\n    if let Err(error) = real_main()"),
        "Windows-subsystem startup must install a panic hook before fallible startup"
    );
    assert!(
        source.contains("fn real_main() -> Result<()>"),
        "fallible startup should live behind a non-Result Windows main wrapper"
    );
    assert!(
        source.contains("show_startup_fatal_message_box"),
        "startup failures and panics should show a visible Windows dialog"
    );
    assert!(
        source.contains("MessageBoxW"),
        "Windows-subsystem fatal errors need MessageBoxW because there is no console"
    );
    assert!(
        source.contains("fn crash_report_dir() -> Option<PathBuf>")
            && source.contains(".map(|base| base.join(\"crashes\"))"),
        "crash reports should be written under the platform app state directory"
    );
    assert!(source.contains("env!(\"CARGO_PKG_VERSION\")"));
    assert!(manifest.contains("\"Win32_UI_WindowsAndMessaging\""));
}

#[test]
fn linux_build_uses_real_gui_instead_of_failure_stub() {
    let source = main_source();
    let manifest = app_manifest_source();

    assert!(
        !source.contains("#[cfg(not(windows))]\nfn main() -> std::process::ExitCode"),
        "Linux builds must launch the same egui/wgpu desktop viewer, not a failure stub"
    );
    assert!(
        source.contains("mod app"),
        "the GUI implementation should be compiled cross-platform"
    );
    assert!(
        !source.contains("#[cfg(windows)]\nmod app"),
        "app module must not be hidden behind cfg(windows)"
    );
    assert!(
        manifest.contains("features = [\"wgpu\", \"default_fonts\", \"x11\", \"wayland\"]"),
        "Linux GUI builds need eframe's x11 and wayland backends enabled"
    );
}

#[test]
fn linux_window_identity_matches_desktop_metadata() {
    let main_source = main_source();
    let bootstrap_source = app_bootstrap_source();
    let build_deb = linux_build_deb_source();
    let package_workflow = package_workflow_source();
    let metainfo = linux_metainfo_source();
    let desktop = linux_desktop_source();

    assert!(
        main_source.contains("LINUX_DESKTOP_APP_ID: &str = \"ai.occlutrace.OccluView\"")
            && bootstrap_source.contains(".with_app_id(crate::LINUX_DESKTOP_APP_ID)"),
        "Wayland app_id should match the installed desktop file id"
    );
    assert!(build_deb.contains("ai.occlutrace.OccluView.desktop"));
    assert!(metainfo
        .contains("<launchable type=\"desktop-id\">ai.occlutrace.OccluView.desktop</launchable>"));
    assert!(package_workflow
        .contains("desktop-file-validate install/linux/ai.occlutrace.OccluView.desktop"));
    assert!(!package_workflow.contains("desktop-file-validate install/linux/occluview.desktop"));
    assert!(desktop.contains("StartupNotify=true"));
    assert!(bootstrap_source.contains("capture_activation_token"));
    assert!(include_str!("../single_instance/activation.rs").contains("xdg_activation"));
}

#[test]
fn linux_desktop_state_uses_xdg_paths() {
    let app_paths = include_str!("../app_paths.rs");
    let single_instance_unix = include_str!("../single_instance/unix.rs");

    assert!(
        app_paths.contains("XDG_STATE_HOME") && app_paths.contains(".local/state"),
        "recent files and crash reports on Linux should use XDG state directories"
    );
    assert!(
        single_instance_unix.contains("XDG_RUNTIME_DIR"),
        "Linux single-instance IPC should prefer XDG_RUNTIME_DIR"
    );
    assert!(
        single_instance_unix.contains("UnixListener")
            && single_instance_unix.contains("UnixStream"),
        "Linux single-instance handoff should use Unix domain sockets"
    );
}

#[test]
fn public_linux_copy_is_not_left_as_windows_only() {
    let app_manifest = app_manifest_source();
    let live_viewport = include_str!("../live_viewport.rs");
    let about = function_source(app_dialogs_source(), "pub(super) fn show_about_window");
    let ci = ci_workflow_source();

    assert!(!app_manifest.contains("Windows-only"));
    assert!(!live_viewport.contains("Windows desktop app"));
    assert!(!about.contains("Native Windows viewer for fast scan inspection"));
    assert!(!ci.contains("Build the Windows-only crates (shell, app)"));
}

#[test]
fn third_party_notices_stay_generated_and_gated() {
    let ci = ci_workflow_source();
    let script = include_str!("../../../../scripts/gen-third-party.sh");

    // The attribution file is generated, so the only honest state is
    // "regenerates identically in CI": pin the generator, fail on drift.
    assert!(
        ci.contains("cargo install cargo-about --version 0.8.4 --locked"),
        "CI should install the pinned cargo-about"
    );
    assert!(
        ci.contains("git diff --exit-code -- THIRD-PARTY-NOTICES.md"),
        "CI should fail when the committed notices drift from the lockfile"
    );
    // The generator polices its own output: the font licenses whose
    // notice-retention terms forced this file into existence must be
    // present, and no first-party crate may attribute itself.
    assert!(script.contains("SIL OPEN FONT LICENSE"));
    assert!(script.contains("UBUNTU FONT LICENCE"));
    assert!(script.contains("first-party crate leaked"));
}

#[test]
fn every_windows_artifact_ships_the_license_set() {
    let wxs = msi_wxs_source();
    let package = package_workflow_source();
    let lifecycle = include_str!("../../../../install/test-msi-lifecycle.ps1");

    // Distributing the statically linked dependencies obliges shipping their
    // notices; both Windows artifacts must carry the same three files.
    for file_id in [
        "filLicenseFile",
        "filNoticeFile",
        "filThirdPartyNotices",
        "filThirdPartyNoticesNative",
    ] {
        assert!(wxs.contains(file_id), "MSI must install {file_id}");
    }
    assert!(
        package.contains("Copy-Item ./THIRD-PARTY-NOTICES.md")
            && package.contains("Copy-Item ./THIRD-PARTY-NOTICES-NATIVE.md"),
        "the portable ZIP must ship the third-party notices, native ones included"
    );
    assert!(
        wxs.contains("<ComponentRef Id=\"cmpThirdPartyNoticesNative\" />"),
        "a component that is declared but never referenced installs nothing"
    );
    assert!(
        lifecycle.contains("THIRD-PARTY-NOTICES.md"),
        "the MSI lifecycle smoke should verify the notices land on disk"
    );
}

#[test]
fn the_deb_ships_and_gates_the_license_set() {
    let build = linux_build_deb_source();
    let check = linux_check_deb_source();
    let copyright = include_str!("../../../../install/linux/copyright");

    assert!(
        build.contains("usr/share/doc/occluview/NOTICE")
            && build.contains("usr/share/doc/occluview/THIRD-PARTY-NOTICES.md")
            && build.contains("usr/share/doc/occluview/THIRD-PARTY-NOTICES-NATIVE.md"),
        "the deb must install the Apache NOTICE and both attribution files"
    );
    assert!(
        check.contains("usr/share/doc/occluview/NOTICE")
            && check.contains("usr/share/doc/occluview/THIRD-PARTY-NOTICES.md")
            && check.contains("SIL OPEN FONT LICENSE"),
        "check-deb.sh must fail a package that lost the license set"
    );
    assert!(
        copyright.contains("THIRD-PARTY-NOTICES.md"),
        "the DEP-5 copyright should point at the shipped attribution file"
    );
}

#[test]
fn the_viewer_answers_version_before_any_windowing() {
    let bootstrap = app_bootstrap_source();

    let version_exit = bootstrap.find("if args.version {");
    let single_instance = bootstrap.find("SingleInstance::acquire");
    assert!(
        version_exit.is_some() && single_instance.is_some(),
        "both the version early-exit and the single-instance handshake should exist"
    );
    // --version must never focus a running instance or open a window; the
    // early exit has to sit before the single-instance handshake.
    assert!(version_exit < single_instance);
    assert!(app_module_source().contains("\"--version\" | \"-V\""));
}

#[test]
fn the_release_page_quotes_the_changelog_and_attests_the_sboms() {
    let package = package_workflow_source();

    assert!(
        package.contains(r#"awk -v ver="$version""#) && package.contains("CHANGELOG.md"),
        "release notes should carry this version's changelog section verbatim"
    );
    assert!(
        package.contains("dist/sbom-*.json"),
        "the SBOMs should be provenance-attested alongside the installers"
    );
}

#[test]
fn the_release_path_can_be_rehearsed_and_refuses_to_ship_a_broken_artifact() {
    let package = package_workflow_source();
    let ci = ci_workflow_source();

    // Five single points of failure in a row, each of which fires only after
    // the tag is public. They need a rehearsal that is not a release.
    assert!(
        package.contains("release_dry_run"),
        "the packaging path must be runnable without cutting a release"
    );
    assert!(
        package.contains("if: ${{ !inputs.release_dry_run }}"),
        "a rehearsal must stop short of publishing"
    );
    assert!(
        package.matches("timeout-minutes:").count() >= 3,
        "every packaging job needs a budget; the default is six hours"
    );

    // --override-filename takes a base name. Passing a full file name produced
    // sbom-windows.json.json, and the move that followed failed the release.
    assert!(!package.contains("--override-filename sbom-windows.json"));
    assert!(!package.contains("--override-filename sbom-linux.json"));
    for sbom in [
        "crates/occluview-app/sbom-windows.json",
        "crates/occluview-app/sbom-linux.json",
    ] {
        assert!(
            package.contains(sbom),
            "the SBOM must be taken from the shipped viewer's crate, not the workspace root"
        );
    }
    assert!(
        package.matches("not the shipped viewer").count() == 2,
        "both SBOM steps must check which component they describe"
    );

    // The old guard read a variable scoped to another step, so it failed a
    // release whose artifacts were correctly signed.
    assert!(!package.contains("Authenticode signing is required for tagged releases"));
    assert!(package.contains("No signing material resolved for tagged release."));

    // The signing key and the key compiled into the updater must agree, or
    // every installed copy silently stops updating.
    assert!(package.contains("UPDATE_PUBKEY"));
    assert!(package.contains("crates/occluview-update/src/lib.rs"));
    assert!(package.contains("minisign -V -P \"$pubkey\""));

    // An empty changelog section would publish a release page that says
    // nothing about what changed.
    assert!(package.contains("has no '## $version' section"));

    // The lockfile is an input to every gate, not a thing CI may update.
    assert!(
        ci.matches("--locked").count() >= 6,
        "every cargo invocation in CI should pin the committed lockfile"
    );
    // The shipped feature combination has to be compiled by something.
    assert!(
        ci.contains("--all-features --all-targets --locked -- -D warnings")
            && ci.contains("cargo test -p occluview-hps -p occluview-formats --all-features"),
        "CI must build the private-hps-key combination that actually ships"
    );
}

#[test]
fn the_fuzz_targets_are_actually_buildable_and_ci_actually_runs_them() {
    // Every fuzz step in CI had been failing since it was written, in three
    // independent ways, and the badge never showed it because nobody read the
    // job. The manifest lacked the `cargo-fuzz = true` marker, so `cargo fuzz`
    // refused it outright; the `[[bin]]` stanzas had been deleted on the
    // premise that cargo auto-discovers `fuzz_targets/` (it discovers only
    // `src/bin/`); and the steps ran with `working-directory: fuzz`, which
    // makes cargo-fuzz look for `fuzz/fuzz/Cargo.toml`.
    let manifest = include_str!("../../../../fuzz/Cargo.toml");
    let ci = ci_workflow_source();
    let runner = include_str!("../../../../scripts/run-fuzz.sh");

    assert!(
        manifest.contains("cargo-fuzz = true"),
        "cargo-fuzz refuses a manifest without its metadata marker"
    );
    for target in ["dispatch", "hps_parser", "stl", "ply", "glb"] {
        assert!(
            manifest.contains(&format!("name = \"{target}\"")),
            "fuzz target {target} needs a [[bin]] stanza to build at all"
        );
        assert!(
            manifest.contains(&format!("path = \"fuzz_targets/{target}.rs\"")),
            "fuzz target {target} needs its source path declared"
        );
        assert!(
            ci.contains(&format!("run-fuzz.sh {target} 60")),
            "the smoke job should fuzz {target}"
        );
        assert!(
            ci.contains(&format!("run-fuzz.sh {target} 300")),
            "the weekly deep job should fuzz {target}"
        );
    }
    assert!(
        !ci.contains("working-directory: fuzz"),
        "cargo-fuzz resolves <cwd>/fuzz/Cargo.toml and must run from the repo root"
    );
    // The seeds are the point: without them the budget goes on rediscovering
    // magic numbers, and the writable corpus must never be the tracked one.
    assert!(runner.contains("fuzz/seeds/$target"));
    assert!(runner.contains("fuzz/corpus/$target"));
    assert!(runner.contains("-dict=$dictionary"));
    assert!(
        ci.contains("path: fuzz/corpus"),
        "the corpus should carry between runs or every run starts from zero"
    );
}

#[test]
fn no_scan_path_reaches_the_crash_report() {
    // `write_crash_report` dumps the log ring buffer to a text file, and a
    // dental scan's path is the case it belongs to. The moment someone is asked
    // to attach that report to a public issue, they attach patient identifiers.
    let bootstrap = app_bootstrap_source();

    assert!(
        !bootstrap.contains("files = ?args.files"),
        "startup must log the shape of the session, not the paths in it"
    );
    assert!(
        bootstrap.contains("file_count = args.files.len()")
            && bootstrap.contains("file_extensions("),
        "startup should log how many files and of which kinds"
    );
    assert!(
        bootstrap.contains("fn write_crash_report"),
        "this test is about what the crash report can contain"
    );

    // The Explorer handlers run over whatever folder is on screen, so the same
    // rule applies to the thumbnail crate.
    let thumbnail = include_str!("../../../occluview-thumbnail/src/render_thumb/mod.rs");
    assert!(
        !thumbnail.contains("path = %path.display()"),
        "the thumbnail path must not be logged; the extension is enough to diagnose"
    );
}

#[test]
fn the_statically_linked_cpp_components_are_attributed() {
    // `THIRD-PARTY-NOTICES.md` is generated from `Cargo.lock` and therefore
    // covers the Rust graph only. The shipped binaries also statically link a
    // C++ geometry kernel that `manifold-csg-sys` fetches and builds, plus the
    // two libraries Manifold's own CMake fetches. Apache-2.0 section 4 obliges
    // anyone redistributing those to carry their notices, and this is a product
    // that is sold.
    let native = include_str!("../../../../THIRD-PARTY-NOTICES-NATIVE.md");
    for component in ["Manifold", "oneTBB", "Clipper2"] {
        assert!(
            native.contains(component),
            "{component} is linked into the binaries and must be attributed"
        );
    }
    assert!(
        native.contains("Apache License") && native.contains("Boost Software License"),
        "the notices must carry the license texts, not only the names"
    );
    // The two gaps a reader should not have to discover.
    assert!(
        native.contains("tag, not a commit"),
        "the upstream reference is mutable and that has to be stated"
    );
    assert!(
        native.contains("cargo deny") && native.contains("SBOM"),
        "neither the advisory scan nor the SBOM sees this code; say so"
    );

    let check = linux_check_deb_source();
    assert!(
        check.contains("THIRD-PARTY-NOTICES-NATIVE.md"),
        "the deb gate must fail a package that dropped the native notices"
    );
}
