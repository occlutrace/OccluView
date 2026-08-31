//! Contracts over what the installers write, and over the scripts that build
//! them.
//!
//! Split out of `shell_contract_tests` to hold the workspace's 800-line file
//! budget, which the module's own guard enforces.

use super::{owns_extension, OFFERED_ONLY_EXTENSIONS};

/// Assert that neither installer surface claims `dot_ext` machine-wide.
///
/// Used from `shell_contract_tests` too, which walks every supported extension.
///
/// These are the values a foreign handler owns: the unnamed `ProgID` under the
/// bare extension key, its `DefaultIcon`, and the `ShellEx` providers under
/// both the bare key and `SystemFileAssociations`, which Explorer falls back
/// to when the owning `ProgID` supplies none.
pub(super) fn assert_extension_is_offered_not_owned(wxs: &str, reg: &str, dot_ext: &str) {
    for forbidden in [
        format!("Software\\Classes\\{dot_ext}\">"),
        format!("Software\\Classes\\{dot_ext}\\DefaultIcon"),
        format!("Software\\Classes\\{dot_ext}\\ShellEx"),
        format!("Software\\Classes\\SystemFileAssociations\\{dot_ext}\\ShellEx"),
    ] {
        assert!(
            !wxs.contains(&forbidden),
            "the MSI must not claim {dot_ext} machine-wide: found {forbidden}"
        );
    }
    for forbidden in [
        format!("[HKEY_CLASSES_ROOT\\{dot_ext}]"),
        format!("[HKEY_CLASSES_ROOT\\{dot_ext}\\DefaultIcon]"),
        format!("[HKEY_CLASSES_ROOT\\{dot_ext}\\ShellEx"),
        format!("[HKEY_CLASSES_ROOT\\SystemFileAssociations\\{dot_ext}\\ShellEx"),
    ] {
        assert!(
            !reg.contains(&forbidden),
            "the manual .reg must not claim {dot_ext} machine-wide: found {forbidden}"
        );
    }
}

#[test]
fn dcm_is_offered_to_the_user_and_never_taken_from_medical_dicom() {
    // .dcm is 3Shape's HPS container extension and medical DICOM's at the same
    // time, on the same dental workstation, and this reader rejects DICOM by
    // design. Claiming the extension would hand every CBCT file an icon, a
    // preview and a double-click that all end in an error. Every surface that
    // fires without the user asking for it must therefore stay clear, and every
    // surface the user reaches deliberately must stay present.
    assert_eq!(
        OFFERED_ONLY_EXTENSIONS,
        [occluview_formats::LEGACY_HPS_EXTENSION]
    );
    assert!(!owns_extension("dcm"));
    for owned in ["stl", "ply", "obj", "glb", "hps"] {
        assert!(owns_extension(owned), "{owned} should still be owned");
    }

    let wxs = include_str!("../../../install/occluview.wxs");
    let reg = include_str!("../../../install/occluview-shell-registration.reg");
    assert_extension_is_offered_not_owned(wxs, reg, ".dcm");

    // Still offered: "Open with", Default Apps, and the right-click verb.
    assert!(wxs.contains("Software\\Classes\\.dcm\\OpenWithProgids"));
    assert!(wxs.contains("Software\\Classes\\.dcm\\OpenWithList\\occluview.exe"));
    assert!(wxs.contains("Name=\".dcm\" Type=\"string\" Value=\"MeshFile.HPS\""));
    assert!(wxs.contains("SystemFileAssociations\\.dcm\\shell\\OccluView.Edit\\command"));
    assert!(reg.contains("[HKEY_CLASSES_ROOT\\.dcm\\OpenWithProgids]"));
    assert!(reg.contains("[HKEY_CLASSES_ROOT\\.dcm\\OpenWithList\\occluview.exe]"));
    assert!(reg.contains("\".dcm\"=\"MeshFile.HPS\""));

    // DllRegisterServer applies the same rule; DllUnregisterServer must not,
    // or a build that shipped before this policy keeps its .dcm entries
    // through an uninstall.
    let registration = super::shell_contract_tests::registration_source();
    assert!(registration.contains("if !owns_extension(ext) {\n            continue;"));
    assert!(registration.contains(
        "if owns_extension(ext) {\n                register_extension_fallback(ext, &app_path)?;"
    ));
    assert!(
        !registration.contains("if owns_extension(ext) {\n        let _ = unregister_extension")
    );

    // Linux: the shared MIME database ships `50:application/dicom:*.dcm`. An
    // unweighted glob defaults to 50 too, and OccluView wins that tie, taking
    // the icon, the default application and the thumbnailer for every DICOM.
    let mime = include_str!("../../../install/linux/occluview-mime.xml");
    for pattern in ["*.dcm", "*.DCM"] {
        assert!(
            mime.contains(&format!("<glob pattern=\"{pattern}\" weight=\"40\" />")),
            "{pattern} must sit below application/dicom's weight of 50"
        );
    }
    for pattern in ["*.hps", "*.HPS", "*.ply", "*.PLY"] {
        assert!(
            mime.contains(&format!("<glob pattern=\"{pattern}\" />")),
            "{pattern} is ours alone and needs no weight"
        );
    }

    // Holding the .dcm glob below DICOM keeps medical imaging alone, but on its
    // own it also leaves 3Shape's own export -- the commonest scan file a
    // dental workstation holds -- with no thumbnail and no association at all.
    // Content settles it: `<HPS` in the leading window is an XML HPS container
    // and 128 reserved bytes plus `DICM` is a study, so the two never collide.
    assert!(
        mime.contains("<magic priority=\"60\">")
            && mime.contains("<match type=\"string\" value=\"&lt;HPS\" offset=\"0:256\"/>"),
        "an XML HPS container written as .dcm must be identified by its content"
    );

    // The shared database gives *.obj to application/x-tgif and to model/obj at
    // the same weight, so the verdict is whatever sorts first -- on the
    // distributions tested, the drawing format, which has no thumbnailer of
    // ours and no association with the viewer.
    for pattern in ["*.obj", "*.OBJ"] {
        assert!(
            mime.contains(&format!("<glob pattern=\"{pattern}\" weight=\"60\" />")),
            "{pattern} must outweigh application/x-tgif's claim on the extension"
        );
    }
}

#[test]
fn installer_refreshes_shell_association_cache_after_registry_changes() {
    let registration = super::shell_contract_tests::registration_source();
    let app_bootstrap = include_str!("../../occluview-app/src/app_bootstrap.rs");
    let app_state = include_str!("../../occluview-app/src/app/state.rs");
    let wxs = include_str!("../../../install/occluview.wxs");

    assert!(registration.contains("SHChangeNotify"));
    assert!(registration.contains("SHCNE_ASSOCCHANGED"));
    assert!(registration.contains("SHCNF_IDLIST"));
    assert!(registration.contains("notify_shell_associations_changed();"));
    assert!(app_state.contains("\"--shell-refresh\""));
    assert!(app_bootstrap.contains("notify_shell_associations_changed"));
    assert!(wxs.contains("Id=\"RefreshShellAssociationsInstall\""));
    assert!(wxs.contains("Id=\"RefreshShellAssociationsUninstall\""));
    assert!(wxs.contains("FileKey=\"filOccluViewExe\""));
    assert!(wxs.contains("ExeCommand=\"--shell-refresh\""));
    assert!(wxs.contains("After=\"WriteRegistryValues\""));
    assert!(wxs.contains("After=\"RemoveRegistryValues\""));
    assert!(!wxs.contains("filOccluViewCli"));

    for action_id in [
        "RefreshShellAssociationsInstall",
        "RefreshShellAssociationsUninstall",
    ] {
        let action = wxs.split("<CustomAction").find_map(|candidate| {
            candidate
                .split_once("/>")
                .filter(|(element, _)| element.contains(&format!("Id=\"{action_id}\"")))
                .map(|(element, _)| element)
        });
        assert!(action.is_some(), "missing {action_id} CustomAction");
        let Some(action) = action else { return };

        assert!(
            action.contains("Impersonate=\"yes\""),
            "{action_id} must refresh Explorer as the installing user"
        );
        assert!(
            action.contains("TerminalServerAware=\"yes\""),
            "{action_id} must target the installing user's terminal-server session"
        );
    }
}

#[test]
fn candidate_package_builds_are_lockfile_strict() {
    // Removing --locked from any of these artifact-producing commands lets a
    // package build resolve a different dependency graph than Cargo.lock.
    let deb_build = include_str!("../../../install/linux/build-deb.sh");
    let msi_build = include_str!("../../../install/build-msi.ps1");
    let windows_build = include_str!("../../../scripts/build-windows-msvc.sh");
    let package_workflow = include_str!("../../../.github/workflows/package-msi.yml");

    let deb_command = deb_build
        .lines()
        .find(|line| line.trim_start().starts_with("cargo build "));
    assert!(deb_command.is_some(), "Debian release Cargo build command");
    let Some(deb_command) = deb_command else {
        return;
    };
    assert!(shell_command_tokens(deb_command).contains(&"--locked"));

    for args_name in ["$cargoArgs", "$shellCargoArgs"] {
        let array_start = format!("{args_name} = @(");
        let args = msi_build
            .split_once(&array_start)
            .and_then(|(_, remaining)| remaining.split_once("\n    )"))
            .map(|(args, _)| args);
        assert!(args.is_some(), "{args_name} Cargo command arguments");
        let Some(args) = args else { return };
        assert!(
            args.lines()
                .any(|line| matches!(line.trim(), "\"--locked\"," | "\"--locked\"")),
            "{args_name} must pass --locked to its Cargo build"
        );
    }

    let xwin_commands: Vec<_> = windows_build
        .split("\n\n")
        .filter(|block| block.trim_start().starts_with("cargo xwin build"))
        .collect();
    assert_eq!(xwin_commands.len(), 2, "two native MSI xwin builds");
    for command in xwin_commands {
        assert!(
            shell_command_tokens(command).contains(&"--locked"),
            "each cargo xwin build command must pass --locked"
        );
    }

    let provider_tests: Vec<_> = package_workflow
        .lines()
        .filter(|line| {
            line.contains("cargo test")
                && line.contains("-p occluview-hps --features private-hps-key")
                && line.contains("runtime_provider_reads_generated_embedded_key_when_present")
        })
        .collect();
    assert_eq!(provider_tests.len(), 2, "Windows and Linux provider tests");
    for command in provider_tests {
        assert!(
            shell_command_tokens(command).contains(&"--locked"),
            "each private-key provider test must pass --locked"
        );
    }
}

#[test]
fn windows_package_builds_link_the_msvc_runtime_statically() {
    // The MSI invokes occluview.exe during installation.  A package that
    // depends on a separately installed VC++ runtime can therefore roll back
    // before it has shown the operator anything, especially under /qn.
    let msi_build = include_str!("../../../install/build-msi.ps1");
    let windows_build = include_str!("../../../scripts/build-windows-msvc.sh");
    let static_crt_toolchain = include_str!("../../../install/cmake/occluview-static-crt.cmake");

    for (surface, source) in [
        ("native Windows MSI build", msi_build),
        ("Linux-to-Windows MSI build", windows_build),
    ] {
        assert!(
            source.contains("target-feature=+crt-static"),
            "{surface} must not depend on a preinstalled VC++ runtime"
        );
        assert!(
            source.contains("occluview-static-crt.cmake"),
            "{surface} must align CMake-built native dependencies with Rust's static CRT"
        );
    }

    assert!(
        static_crt_toolchain.contains("CMAKE_MSVC_RUNTIME_LIBRARY \"MultiThreaded\""),
        "the CMake overlay must request /MT for native dependencies"
    );
    assert!(
        static_crt_toolchain.contains("OCCLUVIEW_BASE_CMAKE_TOOLCHAIN_FILE"),
        "the CMake overlay must preserve a cargo-xwin compiler toolchain"
    );
}

#[test]
fn windows_package_runtime_audits_cover_the_full_dynamic_crt_family() {
    let msi_build = include_str!("../../../install/build-msi.ps1");
    let windows_build = include_str!("../../../scripts/build-windows-msvc.sh");

    for (surface, source) in [
        ("native Windows MSI build", msi_build),
        ("Linux-to-Windows MSI build", windows_build),
    ] {
        let normalized_source = source.to_ascii_lowercase();
        for import_name in ["VCRUNTIME", "MSVCP", "UCRTBASE", "api-ms-win-crt-"] {
            assert!(
                normalized_source.contains(&import_name.to_ascii_lowercase()),
                "{surface} must reject dynamic CRT import family {import_name}"
            );
        }
    }
}

#[test]
fn linux_runtime_audit_does_not_mask_a_match_under_pipefail() {
    let windows_build = include_str!("../../../scripts/build-windows-msvc.sh");

    assert!(
        !windows_build.contains("grep -Eiq"),
        "grep -q may close early and hide a dynamic CRT import under pipefail"
    );
    assert!(
        windows_build.contains("grep -Ei") && windows_build.contains(">/dev/null"),
        "the import matcher must consume objdump output before checking its status"
    );
}

#[test]
fn major_upgrade_preserves_the_previous_product_until_the_new_install_succeeds() {
    let wxs = include_str!("../../../install/occluview.wxs");

    assert!(
        wxs.contains("Schedule=\"afterInstallInitialize\""),
        "a failed major upgrade must roll the previous product back into place"
    );
    assert!(
        wxs.contains("REMOVE=\"ALL\" AND NOT UPGRADINGPRODUCTCODE"),
        "the old product must not run its uninstall shell refresh during a major upgrade"
    );
}

#[test]
fn windows_package_lifecycle_exercises_a_same_version_major_upgrade() {
    let lifecycle = include_str!("../../../install/test-msi-lifecycle.ps1");
    let workflow = include_str!("../../../.github/workflows/package-msi.yml");

    assert!(
        lifecycle.contains("[string]$SameVersionUpgradeMsiPath = \"\""),
        "the lifecycle smoke needs a distinct same-version upgrade input"
    );
    assert!(
        lifecycle.contains("Upgrading with same-version MSI:"),
        "the lifecycle smoke must execute the same-version package"
    );
    assert!(
        workflow.contains("occluview-msi-same-version-upgrade"),
        "Windows CI must build a same-version MSI with a new ProductCode"
    );
    assert!(
        workflow.contains("-SameVersionUpgradeMsiPath $sameVersionUpgradeMsi.FullName"),
        "Windows CI must pass the same-version MSI to lifecycle smoke"
    );
}

#[test]
fn package_sbom_generation_is_lockfile_guarded() {
    // cargo-cyclonedx 0.5.8 has no --locked option.  The metadata preflight
    // fixes the resolved graph, while the diff guard stops a modified lockfile
    // before the generated SBOM is published beside a package artifact.
    let workflow = include_str!("../../../.github/workflows/package-msi.yml");

    for (step_name, sbom_name, artifact_command) in [
        (
            "Generate SBOM (Windows)",
            "sbom-windows",
            &["Move-Item", "$sbom", "./dist/sbom-windows.json", "-Force"][..],
        ),
        (
            "Generate SBOM (Linux)",
            "sbom-linux",
            &["cp", "\"$sbom\"", "target/deb/sbom-linux.json"][..],
        ),
    ] {
        let run_block = workflow_step_run_block(workflow, step_name);
        assert!(
            run_block.is_some(),
            "missing or unterminated {step_name} run block"
        );
        let Some(run_block) = run_block else { return };

        let install = shell_command_index(
            run_block,
            &[
                "cargo",
                "install",
                "cargo-cyclonedx",
                "--version",
                "0.5.8",
                "--locked",
            ],
        );
        let metadata = shell_command_index(
            run_block,
            &["cargo", "metadata", "--locked", "--format-version", "1"],
        );
        let cyclonedx = shell_command_index(
            run_block,
            &[
                "cargo",
                "cyclonedx",
                "--format",
                "json",
                "--override-filename",
                sbom_name,
            ],
        );
        let lockfile_guard = shell_command_index(
            run_block,
            &["git", "diff", "--exit-code", "--", "Cargo.lock"],
        );
        let artifact = shell_command_index(run_block, artifact_command);

        assert!(
            install < metadata,
            "{step_name} must install the pinned tool first"
        );
        assert_eq!(
            metadata + 1,
            cyclonedx,
            "{step_name} must run locked Cargo metadata immediately before SBOM generation"
        );
        assert!(
            cyclonedx < lockfile_guard && lockfile_guard < artifact,
            "{step_name} must guard Cargo.lock after generation and before publishing the SBOM"
        );
    }
}

#[test]
fn workflow_run_block_requires_a_following_step_boundary() {
    let incomplete_workflow =
        "      - name: Generate SBOM (Windows)\n        run: |\n          cargo cyclonedx";

    assert!(
        workflow_step_run_block(incomplete_workflow, "Generate SBOM (Windows)").is_none(),
        "an unterminated workflow step must not absorb following YAML"
    );
}

fn workflow_step_run_block<'a>(workflow: &'a str, step_name: &str) -> Option<&'a str> {
    let marker = format!("- name: {step_name}");
    let (_, step_and_following) = workflow.split_once(&marker)?;
    let (step, _) = step_and_following.split_once("\n      - ")?;
    step.split_once("run: |\n").map(|(_, run_block)| run_block)
}

fn shell_command_tokens(command: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    for line in command.lines() {
        let line = line.split_once('#').map_or(line, |(prefix, _)| prefix);
        tokens.extend(line.split_whitespace().filter(|token| *token != "\\"));
        if !line.trim_end().ends_with('\\') {
            break;
        }
    }
    tokens
}

fn shell_command_index(block: &str, expected_tokens: &[&str]) -> usize {
    let index = block
        .lines()
        .position(|line| shell_command_tokens(line) == expected_tokens);
    assert!(
        index.is_some(),
        "missing command: {}",
        expected_tokens.join(" ")
    );
    let Some(index) = index else { return 0 };
    index
}
