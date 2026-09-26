use super::*;

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

/// The `AppUserModelID` the process sets must be the one the installed shortcut
/// is tagged with, or the taskbar groups the running viewer under a second,
/// unnamed entry and the jump list disappears.
///
/// This is the value-agreement half of the guard the source-text removal left
/// behind: the constant is compared against the MSI that actually ships, not
/// against a second copy of the string. Runs on the Windows CI job, where
/// `APP_USER_MODEL_ID` exists.
#[cfg(windows)]
#[test]
fn windows_app_identity_value_matches_the_shipped_shortcut() {
    let wxs = msi_wxs_source();
    let expected = format!(
        "<ShortcutProperty Key=\"System.AppUserModel.ID\" Value=\"{APP_USER_MODEL_ID}\" />"
    );
    assert!(
        wxs.contains(&expected),
        "install/occluview.wxs must tag its Start Menu shortcut with the \
         process AppUserModelID {APP_USER_MODEL_ID}"
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
fn the_macos_bundle_ships_the_license_set_and_the_safe_finder_registration() {
    let build = macos_build_app_source();
    let plist = macos_info_plist_source();
    let ci = ci_workflow_source();

    // The statically linked native CSG library and every Rust dependency are
    // redistributed inside the bundle, so the same four notices the MSI and the
    // deb carry must travel with it.
    for notice in [
        "LICENSE",
        "NOTICE",
        "THIRD-PARTY-NOTICES.md",
        "THIRD-PARTY-NOTICES-NATIVE.md",
    ] {
        assert!(
            build.contains(notice),
            "the macOS builder must copy {notice} into the bundle"
        );
    }
    assert!(
        build.contains(r#"resources="$contents/Resources""#) && build.contains("$resources/Legal"),
        "the notices belong inside the bundle, in a predictable Resources/Legal directory"
    );
    assert!(
        build.contains("Helpers/occluview-cli"),
        "the command-line companion ships inside the bundle, not as a second download"
    );
    assert!(
        ci.contains("Legal/$notice"),
        "the macOS CI package step must fail a bundle that lost the license set"
    );
    assert!(
        ci.contains("OccluView-*-aarch64.dmg") && ci.contains("OccluView-*-aarch64.pkg"),
        "the macOS CI step should verify the DMG and the PKG it just built"
    );
    // Developer ID signing is a maintainer gate with credentials this
    // repository does not hold; a local builder that pretended to sign would
    // hide the difference between a test artifact and a release.
    assert!(
        !build.contains("codesign"),
        "the unsigned developer builder must not claim to sign anything"
    );

    // Finder integration covers every format the viewer opens, the legacy HPS
    // `.dcm` container included: `V1_OPEN_EXTENSIONS` has carried that suffix
    // since v1, so a bundle that hides it would disagree with the app's own
    // Open dialog.
    for extension in ["stl", "ply", "obj", "glb", "hps", "dcm"] {
        assert!(
            plist.contains(&format!("<string>{extension}</string>")),
            "the bundle should register .{extension} with Launch Services"
        );
    }
    // `.dcm` is also the medical DICOM suffix, so it may only ever be an opt-in
    // surface. The handler rank is what decides that: `Owner` or `Default`
    // would make OccluView the system-wide handler for every DICOM file, which
    // is the same reason Windows keeps `.dcm` in `OFFERED_ONLY_EXTENSIONS` and
    // registers only its Open-with entries.
    let document_types = plist
        .split("<key>CFBundleDocumentTypes</key>")
        .nth(1)
        .and_then(|rest| rest.split("<key>UTImportedTypeDeclarations</key>").next())
        .unwrap_or_default();
    assert!(
        document_types.contains("<string>ai.occlutrace.occluview.dcm</string>"),
        "the .dcm container must reach Launch Services through CFBundleDocumentTypes"
    );
    assert!(
        document_types.contains("<string>Alternate</string>"),
        "the .dcm offer must stay an alternate handler"
    );
    for claimed_rank in ["<string>Owner</string>", "<string>Default</string>"] {
        assert!(
            !document_types.contains(claimed_rank),
            "OccluView must not claim {claimed_rank} for .dcm: medical DICOM files \
             belong to their own tools"
        );
    }
    // Imported, not exported: OccluView reads these third-party formats and
    // does not define them, and an exported declaration would assert an
    // ownership it does not have over the `.dcm` suffix.
    let imported = plist
        .split("<key>UTImportedTypeDeclarations</key>")
        .nth(1)
        .unwrap_or_default();
    assert!(
        imported.contains("<string>ai.occlutrace.occluview.dcm</string>")
            && imported.contains("<string>dcm</string>"),
        "the .dcm container needs its own imported type declaration and extension tag"
    );
    assert!(
        !plist.contains("UTExportedTypeDeclarations"),
        "these formats are imported; exporting them would claim ownership OccluView lacks"
    );
    assert!(
        plist.contains("<string>occluview</string>") && plist.contains("<string>14.0</string>"),
        "the bundle must name the shipped executable and its macOS 14 floor"
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
