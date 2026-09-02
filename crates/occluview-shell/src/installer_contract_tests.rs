//! Contracts over what the installers write, and over the scripts that build
//! them.
//!
//! Split out of `shell_contract_tests` to hold the workspace's 800-line file
//! budget, which the module's own guard enforces.

use super::{owns_extension, OFFERED_ONLY_EXTENSIONS};

const WINDOWS_DEFAULT_PREVHOST_APPID: &str = "{6D2B5079-2F0B-48DD-AB7F-97CEC514D30B}";

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
fn preview_handler_uses_the_windows_prevhost_registration_from_main() {
    // The verified main build uses Windows' standard low-integrity Prevhost
    // AppID. The installer and self-registration must never own or delete
    // that Windows key.
    let registration = super::shell_contract_tests::registration_source();
    let wxs = include_str!("../../../install/occluview.wxs");
    let reg = include_str!("../../../install/occluview-shell-registration.reg");
    let lifecycle = include_str!("../../../install/test-msi-lifecycle.ps1");
    assert!(registration.contains(WINDOWS_DEFAULT_PREVHOST_APPID));
    assert!(!registration.contains("register_preview_handler_appid"));
    assert!(!registration.contains("unregister_preview_handler_appid"));
    assert!(!registration.contains("PREVHOST_APPID_KEY"));
    assert!(!registration.contains("PREVHOST_SURROGATE_H"));

    assert!(wxs.contains(&format!(
        "<?define PrevhostAppId = \"{WINDOWS_DEFAULT_PREVHOST_APPID}\" ?>"
    )));
    assert!(!wxs.contains("cmpPreviewHostRegistration"));
    assert!(!wxs.contains("Software\\Classes\\AppID\\$(var.PrevhostAppId)"));
    assert!(!wxs.contains("DllSurrogate"));
    assert!(!wxs.contains("DisableLowILProcessIsolation"));

    assert!(reg.contains(&format!("\"AppID\"=\"{WINDOWS_DEFAULT_PREVHOST_APPID}\"")));
    assert!(!reg.contains("[HKEY_LOCAL_MACHINE\\Software\\Classes\\AppID\\"));
    assert!(!reg.contains("DllSurrogate"));
    assert!(!reg.contains("DisableLowILProcessIsolation"));

    assert!(lifecycle.contains("$prevhostAppId = \"{6D2B5079-2F0B-48DD-AB7F-97CEC514D30B}\""));
    assert!(lifecycle.contains("preview low-integrity isolation override"));
    assert!(!lifecycle.contains("$previewAppIdPath"));
    assert!(!lifecycle.contains("DllSurrogate"));
}

#[test]
fn legacy_preview_host_migration_is_opt_in_and_checksum_pinned() {
    let workflow = include_str!("../../../.github/workflows/package-msi.yml");
    let lifecycle = include_str!("../../../install/test-msi-lifecycle.ps1");

    assert!(workflow.contains("legacy_msi_run_id"));
    assert!(workflow.contains("legacy_msi_sha256"));
    assert!(workflow.contains("Legacy MSI migration inputs must be supplied together."));
    assert!(workflow.contains("Get-FileHash -Algorithm SHA256"));
    assert!(workflow.contains("OCCLUVIEW_LEGACY_MSI_PATH"));
    assert!(workflow.contains("OCCLUVIEW_LEGACY_MSI_RUN_ID: ${{ inputs.legacy_msi_run_id }}"));
    assert!(workflow.contains("OCCLUVIEW_LEGACY_MSI_SHA256: ${{ inputs.legacy_msi_sha256 }}"));
    assert!(workflow.contains("permissions:\n      contents: read\n      actions: read"));

    assert!(lifecycle.contains("[string]$LegacyUpgradeMsiPath = \"\""));
    assert!(lifecycle.contains("Installing pinned legacy MSI:"));
    assert!(lifecycle.contains("legacy preview CLSID AppID"));
    assert!(lifecycle.contains("Migrating pinned legacy MSI:"));
}

fn assert_diagnostic_package_sources(cargo: &str, native_build: &str, msi_build: &str, wxs: &str) {
    assert!(cargo.contains("[profile.release-diagnostic]"));
    assert!(cargo.contains("debug = 2"));
    assert!(cargo.contains("strip = \"none\""));
    assert!(cargo.contains("[profile.release-diagnostic-unwind]"));
    assert!(cargo.contains("inherits = \"release-diagnostic\""));
    assert!(cargo.contains("panic = \"unwind\""));
    for source in [native_build, msi_build] {
        assert!(source.contains("diagnostic"));
        assert!(source.contains("release-diagnostic-unwind"));
        assert!(source.contains("diagnostic-logs"));
        assert!(source.contains("target-feature=+crt-static"));
    }
    for required in [
        "-diagnostic.msi",
        "-diagnostic.manifest.json",
        "IncludeDiagnostics=1",
        "Get-FileHash -Algorithm SHA256",
        "downloadable to repository readers",
        "never published as a GitHub Release asset",
        "occluview_shell.pdb",
        "occluview.pdb",
        "README.txt",
    ] {
        assert!(
            msi_build.contains(required),
            "diagnostic MSI lacks {required}"
        );
    }
    assert!(wxs.contains("<?if $(var.IncludeDiagnostics) = 1 ?>"));
    for component in [
        "cmpDiagnosticAppSymbols",
        "cmpDiagnosticShellSymbols",
        "cmpEnablePreviewDiagnostics",
        "cmpCollectPreviewDiagnostics",
        "cmpDiagnosticReadme",
    ] {
        assert!(
            wxs.contains(component),
            "diagnostic component missing: {component}"
        );
    }
    for forbidden in [
        "ShellEventLogEnabled",
        "LocalDumps",
        "DisableLowILProcessIsolation",
    ] {
        assert!(
            !wxs.contains(forbidden),
            "the diagnostic MSI must not change a system-wide diagnostics or Prevhost policy: {forbidden}"
        );
    }
}

fn assert_diagnostic_collection_contract(
    enable: &str,
    collect: &str,
    diagnostic_readme: &str,
    lifecycle: &str,
) {
    assert!(enable.contains("HKCU:\\Software\\OccluTrace\\OccluView\\Diagnostics"));
    assert!(enable.contains("ShellEventLogEnabled"));
    assert!(!enable.contains("HKLM:"), "diagnostics must stay per-user");
    for required in [
        "shell-events.jsonl",
        "preview-failures.jsonl",
        "shell-registration.txt",
        "$arguments = @(\"query\", $Key, \"/s\")",
        "& reg.exe @arguments",
        "Compress-Archive",
    ] {
        assert!(
            collect.contains(required),
            "diagnostic collector lacks {required}"
        );
    }
    for forbidden in ["reg.exe add", "reg.exe delete", "regsvr32", "ie4uinit"] {
        assert!(
            !collect.contains(forbidden),
            "collector must remain read-only: {forbidden}"
        );
    }
    assert!(!collect.contains("Start-Process -Verb RunAs"));
    for required in [
        "no admin",
        "$env:USERPROFILE\\Desktop",
        "no source mesh/path",
        "No dumps are collected automatically",
    ] {
        assert!(
            diagnostic_readme.contains(required),
            "diagnostic readme lacks {required}"
        );
    }
    for required in [
        "[switch]$Diagnostic",
        "function Assert-DiagnosticPayload",
        "function Assert-DiagnosticSwitchUnchanged",
        "README.txt",
    ] {
        assert!(
            lifecycle.contains(required),
            "lifecycle contract lacks {required}"
        );
    }
}

fn assert_diagnostic_workflow(workflow: &str) {
    for required in [
        "windows_configuration:",
        "diagnostic",
        "$buildArgs = @{",
        "Configuration = $configuration",
        "SignMode = \"auto\"",
        "$configuration = \"release\"",
        "$env:GITHUB_EVENT_NAME -eq \"workflow_dispatch\"",
        "inputs.windows_configuration != 'diagnostic'",
        "Diagnostic MSI lifecycle smoke",
        "-MsiPath $diagnosticMsi.FullName -Diagnostic",
    ] {
        assert!(
            workflow.contains(required),
            "diagnostic workflow lacks {required}"
        );
    }
    let linux_job = workflow
        .split_once("  linux-package:\n")
        .map(|(_, remaining)| {
            remaining
                .split_once("  publish:\n")
                .map_or(remaining, |(job, _)| job)
        });
    assert!(linux_job.is_some(), "missing Linux package job");
    let Some(linux_job) = linux_job else { return };
    assert!(
        linux_job.contains("if: inputs.windows_configuration != 'diagnostic'"),
        "a diagnostic dispatch must remain Windows-only"
    );
}

#[test]
fn private_diagnostic_msi_can_use_a_monotonic_package_version_without_relabeling_the_release() {
    let workflow = include_str!("../../../.github/workflows/package-msi.yml");

    for required in [
        "windows_msi_version:",
        "private diagnostic MSI product version",
        "$requestedMsiVersion",
        "if ($configuration -ne \"diagnostic\")",
        "$buildArgs = @{",
        "Configuration = $configuration",
        "SignMode = \"auto\"",
        "$buildArgs.Version = $requestedMsiVersion",
    ] {
        assert!(
            workflow.contains(required),
            "private diagnostic package-version contract lacks {required:?}"
        );
    }
}

#[test]
fn diagnostic_msi_is_opt_in_contains_symbols_and_keeps_the_standard_package_path_unchanged() {
    // A diagnostic installer is a non-release investigation tool, not a
    // second customer release channel. It must carry symbols and the opt-in
    // shell feature, while the ordinary release/debug profile names and
    // payloads continue through their existing path.
    let cargo = include_str!("../../../Cargo.toml");
    let native_build = include_str!("../../../scripts/build-windows-msvc.sh");
    let msi_build = include_str!("../../../install/build-msi.ps1");
    let wxs = include_str!("../../../install/occluview.wxs");
    let workflow = include_str!("../../../.github/workflows/package-msi.yml");
    let lifecycle = include_str!("../../../install/test-msi-lifecycle.ps1");
    let repo_root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let enable = std::fs::read_to_string(format!(
        "{repo_root}/install/diagnostics/Enable-PreviewDiagnostics.ps1"
    ))
    .unwrap_or_default();
    let collect = std::fs::read_to_string(format!(
        "{repo_root}/install/diagnostics/Collect-PreviewDiagnostics.ps1"
    ))
    .unwrap_or_default();

    let diagnostic_readme =
        std::fs::read_to_string(format!("{repo_root}/install/diagnostics/README.txt"))
            .unwrap_or_default();
    assert_diagnostic_package_sources(cargo, native_build, msi_build, wxs);
    assert_diagnostic_collection_contract(&enable, &collect, &diagnostic_readme, lifecycle);
    assert_diagnostic_workflow(workflow);
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
fn release_msi_builds_the_preview_dll_from_the_pinned_working_shell_source() {
    // The viewer stays on the current dependency graph, but Explorer loads a
    // separate COM DLL. Its release payload must therefore come from the
    // known-good shell revision rather than a hand-copied old binary.
    let msi_build = include_str!("../../../install/build-msi.ps1");
    let workflow = include_str!("../../../.github/workflows/package-msi.yml");

    for required in [
        "$referenceShellRevision = \"659725632dffcdf14d62724743f35f1689602bbc\"",
        "git -C $repoRoot cat-file -e",
        "git -C $repoRoot worktree add --detach",
        "Set-ReferenceShellPackageVersion",
        "$referenceShellCargoArgs = @(",
        "\"--locked\",",
        "\"--profile\", \"release-unwind\"",
        "$referenceShellBuildDir",
        "occluview_shell.dll",
        "VersionInfo.FileVersion",
        "does not match MSI version",
    ] {
        assert!(
            msi_build.contains(required),
            "reference shell MSI build lacks {required:?}"
        );
    }
    assert!(
        msi_build.contains("if ($Configuration -eq \"release\")"),
        "only the ordinary release MSI may replace its shell DLL with the pinned working source"
    );
    assert!(
        workflow.contains("fetch-depth: 0"),
        "Windows packaging must fetch the pinned working shell revision before building the MSI"
    );
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

    for required_probe in [
        "function Find-DumpBin",
        "vswhere.exe",
        "Microsoft.VCToolsVersion.default.txt",
        "Hostx64\\x64\\dumpbin.exe",
    ] {
        assert!(
            msi_build.contains(required_probe),
            "the native MSI build must locate dumpbin without assuming Visual Studio tools are on PATH: {required_probe}"
        );
    }

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
fn windows_package_lifecycle_allows_only_monotonic_major_upgrades() {
    let lifecycle = include_str!("../../../install/test-msi-lifecycle.ps1");
    let workflow = include_str!("../../../.github/workflows/package-msi.yml");
    let preview_smoke = include_str!("../../../install/test-preview-handler.ps1");

    assert!(
        lifecycle.contains("[string]$DowngradeMsiPath = \"\""),
        "the lifecycle smoke needs an explicit downgrade probe"
    );
    assert!(
        lifecycle.contains("Attempting blocked downgrade MSI:"),
        "the lifecycle smoke must prove that an older package cannot replace a newer one"
    );
    assert!(
        workflow.contains("-DowngradeMsiPath $releaseMsi.FullName"),
        "Windows CI must pass the current package as the post-upgrade downgrade probe"
    );
    assert!(
        !workflow.contains("occluview-msi-same-version-upgrade"),
        "Windows CI must never exercise equal-version major upgrades"
    );
    assert!(
        !lifecycle.contains("SameVersionUpgradeMsiPath"),
        "the lifecycle smoke must not retain an equal-version upgrade path"
    );
    assert!(
        lifecycle.contains("Invoke-MsiExecExpectFailure"),
        "the downgrade probe must fail explicitly instead of relying on a later registry assertion"
    );
    assert!(
        lifecycle.contains("ExitCode -ne 1603"),
        "the downgrade probe must require Windows Installer's expected blocking result"
    );
    assert!(
        lifecycle.contains("Start-ActivePreviewHost"),
        "the MSI lifecycle smoke must upgrade while the old preview surrogate is live"
    );
    assert!(
        lifecycle.contains("Stop-ActivePreviewHost"),
        "the preview-holder process must be cleaned up after the upgrade probe"
    );
    assert!(
        lifecycle.contains("\"-HoldOpenSeconds\", \"90\""),
        "the preview surrogate must remain live for the actual upgrade window"
    );
    assert!(
        lifecycle.contains(r#"('"{0}"' -f $previewSmokePath)"#),
        "the preview-holder must pass its script path as one PowerShell argument"
    );
    assert!(
        !lifecycle.contains(r#"('\"{0}\"' -f $previewSmokePath)"#),
        "the preview-holder must not put literal backslashes into the script-path quotes"
    );
    assert!(
        preview_smoke.contains("[int]$HoldOpenSeconds = 0"),
        "the preview smoke must support holding an activated COM preview open"
    );
    assert!(
        preview_smoke.contains("PREVIEW_HOLD_READY"),
        "the lifecycle smoke needs a positive signal that the surrogate is active before upgrade"
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
