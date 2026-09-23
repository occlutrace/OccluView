use super::*;

#[test]
fn windows_app_reports_startup_and_panic_failures() {
    let source = app_bootstrap_source();
    let manifest = app_manifest_source();

    assert!(
        source.contains("install_panic_hook();")
            && source.contains("if let Err(error) = real_main()"),
        "Windows-subsystem startup must install a panic hook before fallible startup"
    );
    assert!(
        source.contains("std::process::exit(1);"),
        "a failed GUI startup must return a failure status instead of silently succeeding"
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
    let binary = main_source();
    let library = lib_source();
    let manifest = app_manifest_source();

    assert!(
        !binary.contains("#[cfg(not(windows))]\nfn main() -> std::process::ExitCode"),
        "Linux builds must launch the same egui/wgpu desktop viewer, not a failure stub"
    );
    assert!(
        library.contains("mod app;"),
        "the GUI implementation should live behind the library boundary"
    );
    assert!(
        !library.contains("#[cfg(windows)]\nmod app"),
        "app module must not be hidden behind cfg(windows)"
    );
    assert!(
        manifest.contains("features = [\"wgpu\", \"default_fonts\", \"x11\", \"wayland\"]"),
        "Linux GUI builds need eframe's x11 and wayland backends enabled"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn linux_window_identity_value_matches_desktop_metadata() {
    assert_eq!(
        LINUX_DESKTOP_APP_ID, "ai.occlutrace.OccluView",
        "Wayland app_id value must match the installed desktop file id"
    );
}

#[cfg(windows)]
#[test]
fn windows_app_identity_value_matches_shell_registration() {
    assert_eq!(
        APP_USER_MODEL_ID, "OccluTrace.OccluView",
        "AppUserModelID value must match the shell registration"
    );
}

#[test]
fn platform_identity_values_are_pinned_unconditionally() {
    // The cfg-gated asserts above only run on their platform; pin both
    // values everywhere so cross-platform drift cannot hide.
    assert!(
        lib_source().contains("LINUX_DESKTOP_APP_ID: &str = \"ai.occlutrace.OccluView\""),
        "Wayland app_id value must match the installed desktop file id"
    );
    assert!(
        lib_source().contains("APP_USER_MODEL_ID: &str = \"OccluTrace.OccluView\""),
        "AppUserModelID value must match the shell registration"
    );
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
fn the_release_page_quotes_the_changelog_and_attests_the_sboms() {
    let package = package_workflow_source();

    assert!(
        package.contains(r#"awk -v version="$version""#)
            && package.contains(r#"changelog_section="$(mktemp)""#)
            && package.contains(r#"CHANGELOG.md > "$changelog_section""#),
        "release notes should be built from the matching changelog section"
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

    // Authenticode remains optional; minisign protects the update channel.
    assert!(!package.contains("Authenticode signing is required for tagged releases"));
    assert!(!package.contains("No signing material resolved for tagged release."));
    assert!(package.contains("No Authenticode certificate configured"));

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
fn the_fuzz_manifest_declares_every_target_and_ci_runs_them() {
    // The wiring is what breaks, and nothing else here can check it: building
    // the targets needs a nightly toolchain and a linker pass, which belong in
    // the fuzz job. Every fuzz step in CI failed from the day it was written,
    // three ways at once, and the badge never showed it because nobody read
    // the job. No `cargo-fuzz = true` marker, so `cargo fuzz` refused the
    // manifest; no `[[bin]]` stanzas, on the premise that cargo auto-discovers
    // `fuzz_targets/` (it discovers only `src/bin/`); and
    // `working-directory: fuzz`, which sends cargo-fuzz looking for
    // `fuzz/fuzz/Cargo.toml`.
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

    // The crate is outside the workspace, so no gate here resolves its
    // lockfile, and `cargo fuzz` does not pass --locked. It had fallen a
    // dependency behind with nothing to say so.
    assert!(
        ci.contains("cargo check --manifest-path fuzz/Cargo.toml --locked"),
        "the fuzz job should resolve the fuzz lockfile before it fuzzes"
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

#[test]
fn the_workflows_name_the_package_they_built_instead_of_globbing_for_it() {
    // `dpkg-deb --info target/deb/*.deb` reads every argument after the first
    // as a control-file name, so a second package in the directory turns the
    // check into an error about a missing control file -- or, worse, checks
    // only the oldest one. A fresh runner has exactly one package, which is
    // why this survived; a developer machine has every version ever built.
    //
    // build-deb.sh prints the path it wrote as its last line, so both
    // workflows take it from there.
    for (name, workflow) in [
        ("ci.yml", ci_workflow_source()),
        ("package-msi.yml", package_workflow_source()),
    ] {
        for globbed in [
            "dpkg-deb --info target/deb/*.deb",
            "dpkg-deb --contents target/deb/*.deb",
            "check-deb.sh target/deb/*.deb",
        ] {
            assert!(
                !workflow.contains(globbed),
                "{name} passes a glob where one package belongs: {globbed}"
            );
        }
        assert!(
            workflow.contains("package=\"$(install/linux/build-deb.sh | tail -n 1)\""),
            "{name} should take the package path from the builder"
        );
        assert!(
            workflow.contains("set -o pipefail"),
            "{name} pipes build-deb.sh into tail, so a build failure has to \
             survive the pipe"
        );
    }

    let builder = linux_build_deb_source();
    assert!(
        builder.contains("# Contract: the last line on stdout is the path"),
        "build-deb.sh should say that its last line is the contract the \
         workflows depend on"
    );
}
