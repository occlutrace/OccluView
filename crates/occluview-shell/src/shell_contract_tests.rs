use super::{owns_extension, APP_EXE_NAME, DEDICATED_FILE_ICON_EXTENSIONS, SUPPORTED_EXTENSIONS};
use std::path::Path;

fn canonical_extension(extension: &str) -> &str {
    if extension == "dcm" {
        "hps"
    } else {
        extension
    }
}

pub(super) fn registration_source() -> String {
    [
        include_str!("registration/mod.rs"),
        include_str!("registration/associations.rs"),
        include_str!("registration/clsid.rs"),
        include_str!("registration/paths.rs"),
        include_str!("registration/registry.rs"),
    ]
    .join("\n")
}

#[test]
fn thumbnail_registration_only_includes_implemented_stream_formats() {
    // Comparing the constant with a longhand copy of itself holds whatever the
    // list says, including an extension Explorer would then hand to a reader
    // that does not exist. Registering a format promises the shell that a
    // stream of those bytes produces a thumbnail, and the dispatcher is what
    // keeps the promise.
    // An empty list would pass the loop below vacuously; it cannot be empty,
    // and the compiler says so -- clippy rejects the check as always false.
    for extension in SUPPORTED_EXTENSIONS {
        assert!(
            occluview_formats::probe::by_extension(extension).is_some(),
            "{extension} is registered with Explorer, and no reader claims it"
        );
    }
}

#[test]
fn open_with_targets_the_real_gui_binary_name() {
    // Bound to the manifest that produces the file, the way platform.rs binds
    // the Linux app id to the installed .desktop entry. Against a copy of the
    // constant, renaming the [[bin]] target stays green while "Open with" on
    // every installed machine points at an executable that is gone.
    let stem = APP_EXE_NAME.strip_suffix(".exe").unwrap_or(APP_EXE_NAME);
    let manifest = include_str!("../../occluview-app/Cargo.toml");
    assert!(
        manifest.contains(&format!("name = \"{stem}\"")),
        "APP_EXE_NAME ({APP_EXE_NAME}) must name the [[bin]] target in occluview-app/Cargo.toml"
    );
}

#[test]
fn shell_gpu_tests_use_the_stable_fallback_adapter() {
    let factory = include_str!("offscreen_factory.rs");
    assert!(!factory.contains("Offscreen::new_prefer_hardware()"));
}

#[test]
fn gui_app_uses_windows_subsystem_without_debug_console() {
    let app_main = include_str!("../../occluview-app/src/main.rs");
    assert!(app_main.contains("#![cfg_attr(windows, windows_subsystem = \"windows\")]"));
    assert!(!app_main.contains("not(debug_assertions)"));
}

#[test]
fn gui_app_embeds_brand_icon_and_windows_metadata() {
    let app_manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("../occluview-app");
    assert!(app_manifest.join("build.rs").exists());
    assert!(app_manifest.join("assets/windows/occluview.ico").exists());
    assert!(app_manifest.join("assets/windows/occluview.png").exists());
    assert!(app_manifest.join("assets/windows/occluview.svg").exists());

    let bootstrap = include_str!("../../occluview-app/src/app_bootstrap.rs");
    assert!(bootstrap.contains("with_icon(load_window_icon())"));
    assert!(bootstrap.contains("include_bytes!(\"../assets/windows/occluview.png\")"));

    let build_rs = include_str!("../../occluview-app/build.rs");
    assert!(build_rs.contains("FileDescription"));
    assert!(build_rs.contains("OccluView 3D Viewer"));
    assert!(build_rs.contains("CompanyName"));
    assert!(build_rs.contains("Dental Cloud Technologies"));
    assert!(build_rs.contains("cargo:rustc-link-arg-bin=occluview="));
}

#[test]
fn installer_uses_one_generic_3d_file_type_icon() {
    let icon_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../install/assets/file-icons");
    assert!(icon_dir.join("occluview-3d.ico").exists());
    assert!(icon_dir.join("occluview-3d.svg").exists());

    let wxs = include_str!("../../../install/occluview.wxs");
    for ext in DEDICATED_FILE_ICON_EXTENSIONS {
        let upper = canonical_extension(ext).to_ascii_uppercase();
        assert!(wxs.contains(&format!("MeshFile.{upper}")));
        assert!(wxs.contains("occluview-3d.ico"));
        assert!(!wxs.contains(&format!("occluview-{ext}.ico")));
        assert!(wxs.contains(&format!("Software\\Classes\\MeshFile.{upper}\\DefaultIcon")));
    }
}

#[test]
fn file_type_friendly_names_are_brand_neutral() {
    let wxs = include_str!("../../../install/occluview.wxs");
    let registration = registration_source();
    let reg = include_str!("../../../install/occluview-shell-registration.reg");

    for ext in SUPPORTED_EXTENSIONS {
        let upper = canonical_extension(ext).to_ascii_uppercase();
        let friendly = format!("{upper} File");
        assert!(wxs.contains(&format!("Value=\"{friendly}\"")));
        assert!(reg.contains(&format!("@=\"{friendly}\"")));
        assert!(registration.contains("format_file_type_name(ext)"));
        assert!(!wxs.contains(&format!("Value=\"OccluView {upper} Mesh\"")));
        assert!(!reg.contains(&format!("@=\"OccluView {upper} Mesh\"")));
    }
}

#[test]
fn file_type_progids_are_brand_neutral() {
    let wxs = include_str!("../../../install/occluview.wxs");
    let registration = registration_source();
    let reg = include_str!("../../../install/occluview-shell-registration.reg");

    for ext in SUPPORTED_EXTENSIONS {
        let upper = canonical_extension(ext).to_ascii_uppercase();
        let neutral = format!("MeshFile.{upper}");
        let legacy = format!("OccluView.Mesh.{upper}");

        assert!(wxs.contains(&format!("Software\\Classes\\{neutral}")));
        assert!(wxs.contains(&format!("Value=\"{neutral}\"")));
        assert!(wxs.contains(&format!("Name=\"{neutral}\" Type=\"string\" Value=\"\"")));
        assert!(reg.contains(&format!("[HKEY_CLASSES_ROOT\\{neutral}]")));
        assert!(reg.contains(&format!("@=\"{neutral}\"")));
        assert!(reg.contains(&format!("\"{neutral}\"=\"\"")));
        assert!(registration.contains("format!(\"MeshFile."));
        assert!(!wxs.contains(&legacy));
        assert!(!reg.contains(&legacy));
    }
}

#[test]
fn gui_app_sets_process_app_user_model_id() {
    let app_main = include_str!("../../occluview-app/src/main.rs");
    let bootstrap = include_str!("../../occluview-app/src/app_bootstrap.rs");
    let jump_list = include_str!("../../occluview-app/src/jump_list.rs");

    assert!(app_main.contains("pub(crate) const APP_USER_MODEL_ID"));
    assert!(bootstrap.contains("SetCurrentProcessExplicitAppUserModelID"));
    assert!(bootstrap.contains("set_process_app_user_model_id();"));
    assert!(jump_list.contains("super::APP_USER_MODEL_ID"));
    assert!(!jump_list.contains("const APP_ID"));

    let wxs = include_str!("../../../install/occluview.wxs");
    assert!(wxs.contains("Key=\"System.AppUserModel.ID\""));
    assert!(wxs.contains("Value=\"OccluTrace.OccluView\""));
}

#[test]
#[allow(clippy::too_many_lines)]
fn installer_metadata_tracks_supported_shell_extensions() {
    let wxs = include_str!("../../../install/occluview.wxs");
    let reg = include_str!("../../../install/occluview-shell-registration.reg");

    assert!(wxs.contains("<MajorUpgrade"));
    assert!(wxs.contains("AllowSameVersionUpgrades=\"yes\""));
    assert!(wxs.contains("DowngradeErrorMessage="));
    assert!(wxs.contains("<RemoveFolder Id=\"rmProgramMenuDir\" On=\"uninstall\""));
    assert!(wxs.contains("&quot;[INSTALLFOLDER]occluview.exe&quot; &quot;%1&quot;"));
    assert!(wxs.contains("Software\\RegisteredApplications"));
    assert!(wxs.contains("Software\\OccluTrace\\OccluView\\Capabilities"));
    assert!(wxs.contains("Capabilities\\FileAssociations"));
    assert!(wxs.contains("ThreadingModel\" Type=\"string\" Value=\"Apartment\""));
    assert!(
        wxs.contains("Software\\Microsoft\\Windows\\CurrentVersion\\Shell Extensions\\Approved")
    );
    assert!(wxs.contains("ThumbnailCutoff"));
    assert!(wxs.contains("TypeOverlay"));
    assert!(wxs.contains("Software\\Classes\\Applications\\occluview.exe"));
    assert!(wxs.contains("SupportedTypes"));
    assert!(wxs.contains("OpenWithList\\occluview.exe"));
    // Explorer right-click "Edit in OccluView" on every supported extension,
    // independent of which app owns the default association (HPS included).
    assert!(wxs.contains("shell\\OccluView.Edit"));
    assert!(wxs.contains("Edit in OccluView"));
    for ext in [".stl", ".ply", ".obj", ".glb", ".dcm", ".hps"] {
        assert!(
            wxs.contains(&format!(
                "SystemFileAssociations\\{ext}\\shell\\OccluView.Edit\\command"
            )),
            "missing Edit verb for {ext}"
        );
    }
    assert!(wxs.contains("ApplicationIcon"));
    assert!(wxs.contains("ARPCOMMENTS"));
    assert!(wxs.contains("ARPURLINFOABOUT"));
    assert!(wxs.contains("ARPURLUPDATEINFO"));
    assert!(wxs.contains("ARPHELPLINK"));
    assert!(wxs.contains("ARPNOREPAIR"));
    assert!(wxs.contains("WixUILicenseRtf"));
    assert!(wxs.contains("<?define ProductName = \"OccluView 3D Viewer\" ?>"));
    assert!(Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../install/license.rtf")
        .exists());
    assert!(!wxs.contains("UserChoice"));
    // The WixUI_InstallDir dialog set already injects ARPNOMODIFY=1; defining
    // it in our authoring again is a duplicate-symbol light.exe error
    // (LGHT0091), so the wxs must NOT declare it itself.
    assert!(!wxs.contains("<Property Id=\"ARPNOMODIFY\""));
    assert!(wxs.contains("WixUI_InstallDir"));
    assert!(reg.contains("@=\"\\\"<APP_EXE_PATH>\\\" \\\"%1\\\"\""));
    assert!(reg.contains("[HKEY_LOCAL_MACHINE\\Software\\RegisteredApplications]"));
    assert!(reg.contains("[HKEY_LOCAL_MACHINE\\Software\\OccluTrace\\OccluView\\Capabilities]"));
    assert!(reg.contains(
        "[HKEY_LOCAL_MACHINE\\Software\\OccluTrace\\OccluView\\Capabilities\\FileAssociations]"
    ));
    assert!(reg.contains("\"ThumbnailCutoff\"=dword:00000001"));
    assert!(reg.contains("\"TypeOverlay\"=\"\""));
    assert!(reg.contains("\"ThreadingModel\"=\"Apartment\""));
    assert!(reg.contains(
            "[HKEY_LOCAL_MACHINE\\Software\\Microsoft\\Windows\\CurrentVersion\\Shell Extensions\\Approved]"
        ));
    assert!(reg.contains("[HKEY_CLASSES_ROOT\\Applications\\occluview.exe]"));
    assert!(reg.contains("[HKEY_CLASSES_ROOT\\Applications\\occluview.exe\\SupportedTypes]"));
    assert!(!reg.contains("\\UserChoice"));

    for ext in SUPPORTED_EXTENSIONS {
        let dot_ext = format!(".{ext}");
        let upper = canonical_extension(ext).to_ascii_uppercase();
        let progid = format!("MeshFile.{upper}");
        if owns_extension(ext) {
            assert!(wxs.contains(&format!(
                "Software\\Classes\\{dot_ext}\">\n              <RegistryValue Type=\"string\" Value=\"{progid}\""
            )));
            assert!(wxs.contains(&format!(
                "Software\\Classes\\{dot_ext}\">\n              <RegistryValue Type=\"string\" Value=\"{progid}\" />\n              <RegistryValue Name=\"ThumbnailCutoff\""
            )));
            assert!(wxs.contains(&format!(
                "Software\\Classes\\{dot_ext}\">\n              <RegistryValue Type=\"string\" Value=\"{progid}\" />\n              <RegistryValue Name=\"ThumbnailCutoff\" Type=\"integer\" Value=\"1\" />\n              <RegistryValue Name=\"TypeOverlay\""
            )));
            assert!(wxs.contains(&format!("Software\\Classes\\{dot_ext}\\DefaultIcon")));
            assert!(wxs.contains(&format!("Software\\Classes\\{dot_ext}\\ShellEx")));
        } else {
            super::installer_contract_tests::assert_extension_is_offered_not_owned(
                wxs, reg, &dot_ext,
            );
        }
        assert!(wxs.contains(&format!("Software\\Classes\\{progid}\\ShellEx")));
        assert!(wxs.contains(&format!("Software\\Classes\\{dot_ext}\\OpenWithProgids")));
        assert!(wxs.contains(&format!(
            "Software\\Classes\\{dot_ext}\\OpenWithList\\occluview.exe"
        )));
        assert!(wxs.contains(&format!(
            "Name=\"{dot_ext}\" Type=\"string\" Value=\"{progid}\""
        )));
        assert!(wxs.contains(&format!("Name=\"{progid}\" Type=\"string\" Value=\"\"")));
        assert!(wxs.contains(&format!("Software\\Classes\\{progid}\\DefaultIcon")));
        if owns_extension(ext) {
            assert!(reg.contains(&format!("[HKEY_CLASSES_ROOT\\{dot_ext}]\n@=\"{progid}\"")));
            assert!(reg.contains(&format!(
                "[HKEY_CLASSES_ROOT\\{dot_ext}]\n@=\"{progid}\"\n\"ThumbnailCutoff\"=dword:00000001"
            )));
            assert!(reg.contains(&format!(
                "[HKEY_CLASSES_ROOT\\{dot_ext}]\n@=\"{progid}\"\n\"ThumbnailCutoff\"=dword:00000001\n\"TypeOverlay\"=\"\""
            )));
            assert!(reg.contains(&format!("[HKEY_CLASSES_ROOT\\{dot_ext}\\DefaultIcon]")));
            assert!(reg.contains(&format!("[HKEY_CLASSES_ROOT\\{dot_ext}\\ShellEx")));
        }
        assert!(reg.contains(&format!("[HKEY_CLASSES_ROOT\\{progid}\\ShellEx")));
        assert!(reg.contains(&format!("[HKEY_CLASSES_ROOT\\{dot_ext}\\OpenWithProgids]")));
        assert!(reg.contains(&format!(
            "[HKEY_CLASSES_ROOT\\{dot_ext}\\OpenWithList\\occluview.exe]"
        )));
        assert!(reg.contains(&format!("\"{dot_ext}\"=\"{progid}\"")));
        assert!(reg.contains(&format!("\"{progid}\"=\"\"")));
        assert!(reg.contains(&format!("\"{dot_ext}\"=\"\"")));
    }

    for ext in ["gltf", "3mf"] {
        let dot_ext = format!(".{ext}");
        assert!(!wxs.contains(&format!("Software\\Classes\\{dot_ext}\\ShellEx")));
        assert!(!wxs.contains(&format!("Software\\Classes\\{dot_ext}\\OpenWithProgids")));
        assert!(!wxs.contains(&format!(
            "Software\\Classes\\{dot_ext}\\OpenWithList\\occluview.exe"
        )));
        assert!(!reg.contains(&format!("[HKEY_CLASSES_ROOT\\{dot_ext}\\ShellEx")));
        assert!(!reg.contains(&format!("[HKEY_CLASSES_ROOT\\{dot_ext}\\OpenWithProgids]")));
        assert!(!reg.contains(&format!(
            "[HKEY_CLASSES_ROOT\\{dot_ext}\\OpenWithList\\occluview.exe]"
        )));
    }
}

#[test]
fn ply_thumbnail_registration_has_direct_fallbacks() {
    let wxs = include_str!("../../../install/occluview.wxs");
    let reg = include_str!("../../../install/occluview-shell-registration.reg");
    let smoke = include_str!("../../../install/test-msi-lifecycle.ps1");

    assert!(wxs.contains("Software\\Classes\\.ply\\ShellEx\\$(var.ThumbnailProviderCategory)"));
    assert!(wxs.contains("Software\\Classes\\.ply\\DefaultIcon"));
    assert!(wxs.contains("MeshFile.PLY"));
    assert!(wxs.contains("occluview-3d.ico"));
    assert!(reg.contains(".ply\\ShellEx\\{E357FCCD-A995-4576-B01F-234630154E96}"));
    assert!(reg.contains("[HKEY_CLASSES_ROOT\\.ply]\n@=\"MeshFile.PLY\""));
    assert!(reg.contains("[HKEY_CLASSES_ROOT\\.ply\\DefaultIcon]"));
    assert!(reg.contains("occluview-3d.ico"));
    assert!(smoke.contains(".$ext extension ProgID"));
    assert!(smoke.contains(".$ext extension default icon"));

    let registration = registration_source();
    assert!(registration.contains("register_extension_fallback(ext, &app_path)"));
    assert!(registration.contains("unregister_extension_fallback(ext, app_path.as_ref())"));
    assert!(registration.contains("delete_default_icon_if_occluview(&icon_key)"));
    assert!(registration.contains("set_dword(hk, h!(\"ThumbnailCutoff\"), 1)?;"));
    assert!(registration.contains("register_progid_thumbnail_handler(&progid_string"));
    assert!(registration.contains("register_approved_shell_extension()"));
    assert!(registration.contains("THREADING_MODEL_H: &HSTRING = h!(\"Apartment\")"));
    assert!(registration.contains("set_string(hk, Some(h!(\"TypeOverlay\")), &HSTRING::new())?;"));
}

#[test]
fn package_workflow_runs_installer_lifecycle_smoke() {
    let workflow = include_str!("../../../.github/workflows/package-msi.yml");
    assert!(workflow.contains("./install/test-msi-lifecycle.ps1"));
    assert!(workflow.contains("-UpgradeMsiPath"));
    assert!(workflow.contains("Verify embedded HPS key build path"));
    assert!(workflow.contains("OCCLUVIEW_HPS_EMBEDDED_KEY is required for Package MSI"));
    assert!(workflow.contains("runtime_provider_reads_generated_embedded_key_when_present"));
    assert!(
        workflow
            .matches("git grep --quiet --fixed-strings -e")
            .count()
            >= 2
    );
    assert!(workflow.contains("Select-String -Path $testLog -SimpleMatch"));
    assert!(workflow.contains("grep -Fq -f - \"$test_log\""));

    let build_msi = include_str!("../../../install/build-msi.ps1");
    assert!(build_msi.contains("\"-dBuildDir=$buildDir\""));
    assert!(build_msi.contains("\"-dProductVersion=$Version\""));
    assert!(build_msi.contains("Assert-MsiProductVersion"));
    assert!(build_msi.contains("Private HPS key embedding enabled for this build."));
    assert!(!build_msi.contains("\n    -dBuildDir=$buildDir `"));
    assert!(!build_msi.contains("\n    -dProductVersion=$Version `"));

    let smoke_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../install/test-msi-lifecycle.ps1");
    assert!(smoke_path.exists());

    let smoke = include_str!("../../../install/test-msi-lifecycle.ps1");
    let thumbnail_smoke = include_str!("../../../install/test-thumbnail-provider.ps1");
    assert!(smoke.contains("msiexec.exe"));
    assert!(smoke.contains("UpgradeMsiPath"));
    assert!(smoke.contains("Assert-InstalledRegistry"));
    assert!(smoke.contains("Assert-UninstalledRegistry"));
    assert!(smoke.contains("test-thumbnail-provider.ps1"));
    assert!(thumbnail_smoke.contains("@(16, 32, 96, 256, 1024)"));
    assert!(thumbnail_smoke.contains("ProbeDirect("));
    assert!(thumbnail_smoke.contains("Type.GetTypeFromCLSID"));
    assert!(thumbnail_smoke.contains("IInitializeWithFile"));
    assert!(thumbnail_smoke.contains("IThumbnailProvider"));
    assert!(thumbnail_smoke.contains("HammingDistance"));
    assert!(thumbnail_smoke.contains("using System.Diagnostics;"));
    assert!(thumbnail_smoke.contains("shellWarm"));
    assert!(thumbnail_smoke.contains("warm-shell-stable"));
    assert!(thumbnail_smoke.contains("elapsed="));
    assert!(thumbnail_smoke.contains("$minimumPrimarySpan"));
    assert!(thumbnail_smoke.contains("$minimumSecondarySpan"));
    assert!(thumbnail_smoke.contains("$primarySpan -lt $minimumPrimarySpan"));
    assert!(thumbnail_smoke.contains("New-SmokePly"));
    assert!(thumbnail_smoke.contains("New-SmokeObj"));
    assert!(thumbnail_smoke.contains("New-SmokeHps"));
    assert!(thumbnail_smoke.contains("New-SmokeLegacyHps"));
    assert!(thumbnail_smoke.contains("Assert-ShellProbeSucceeded"));
    assert!(thumbnail_smoke.contains("Assert-MixedFolderBurst"));
    assert!(thumbnail_smoke.contains("occluview-thumbnail-mixed"));
    assert!(thumbnail_smoke.contains("ProbeShellForced"));
    assert!(thumbnail_smoke.contains("WTS_FORCEEXTRACTION"));
    assert!(thumbnail_smoke.contains("$noiseCount"));
    assert!(thumbnail_smoke.contains("$request.Is3d"));
    assert!(thumbnail_smoke.contains("if (-not $request.Is3d)"));
    assert!(thumbnail_smoke.contains("continue"));
    assert!(thumbnail_smoke.contains("-lt 96"));
    assert!(thumbnail_smoke.contains("Thumbnail shell path drifted"));
    assert!(smoke.contains("approved shell extension"));
    assert!(smoke.contains("Assert-RegistryDefaultNotEquals"));
    assert!(smoke.contains("Assert-RegistryNamedValueAbsent"));
    assert!(smoke.contains("Capabilities FileAssociations"));
    assert!(smoke.contains("Applications SupportedTypes"));
    assert!(smoke.contains("OpenWithList"));
    assert!(smoke.contains("$formatProgIds"));
    assert!(smoke.contains("$legacyFormatProgIds"));
    assert!(smoke.contains("$formatIconFiles"));
    assert!(!smoke.contains("Assert-PathAbsent \"HKLM:\\Software\\Classes\\.$ext\\ShellEx"));
    assert!(!smoke.contains(".DisplayName"));
    assert!(
        smoke
            .matches("$codes = @(Find-InstalledProductCodes)")
            .count()
            >= 2
    );
    for ext in SUPPORTED_EXTENSIONS {
        assert!(smoke.contains(&format!("\"{ext}\"")));
        let upper = canonical_extension(ext).to_ascii_uppercase();
        assert!(smoke.contains(&format!("MeshFile.{upper}")));
        assert!(smoke.contains(&format!("OccluView.Mesh.{upper}")));
    }
    assert!(smoke.contains("occluview-3d.ico"));
}

#[test]
fn package_workflow_builds_linux_deb_release_assets() {
    let workflow = include_str!("../../../.github/workflows/package-msi.yml");
    let build_deb = include_str!("../../../install/linux/build-deb.sh");
    let check_deb = include_str!("../../../install/linux/check-deb.sh");

    assert!(workflow.contains("name: build linux deb"));
    assert!(workflow.contains("runs-on: ubuntu-latest"));
    assert!(workflow.contains("OCCLUVIEW_HPS_EMBEDDED_KEY"));
    assert!(workflow.contains("OCCLUVIEW_HPS_EMBEDDED_KEY is required for Package Linux"));
    assert!(workflow.contains("cargo test --locked -p occluview-hps --features private-hps-key"));
    assert!(workflow.contains("install/linux/build-deb.sh"));
    // The package to validate is the one build-deb.sh just named; globbing
    // target/deb hands dpkg-deb a second path as a control-file name.
    assert!(workflow.contains("install/linux/check-deb.sh \"$DEB\""));
    assert!(workflow.contains("appstreamcli validate --no-net"));
    assert!(workflow.contains("xmllint --noout"));
    assert!(workflow.contains("lintian"));
    assert!(workflow.contains("dpkg-deb --info"));
    assert!(workflow.contains("sha256sum \"$(basename \"$deb\")\""));
    assert!(workflow.contains("occluview-linux-package"));
    assert!(workflow.contains("actions/download-artifact"));
    assert!(workflow.contains("*.deb"));
    assert!(workflow.contains("*.sha256"));
    assert!(workflow.contains(r#"Download \`OccluView-Linux.deb\` for the native Linux viewer,"#));
    assert!(workflow.contains("launcher, MIME registration, and thumbnailer."));

    assert!(build_deb.contains("OCCLUVIEW_HPS_EMBEDDED_KEY"));
    assert!(build_deb.contains("occluview-formats/private-hps-key"));
    assert!(build_deb.contains("Private HPS key embedding enabled for this build."));

    assert!(check_deb.contains("dpkg-deb --control"));
    assert!(check_deb.contains("dpkg-deb -x"));
    assert!(check_deb.contains("usr/bin/occluview"));
    assert!(check_deb.contains("usr/bin/occluview-cli"));
    assert!(check_deb.contains("usr/share/thumbnailers/ai.occlutrace.OccluView.thumbnailer"));
    assert!(check_deb.contains("usr/share/doc/occluview/NEWS.gz"));
    assert!(check_deb.contains("usr/share/doc/occluview/changelog.gz"));
    assert!(check_deb.contains("usr/share/man/man1/occluview.1.gz"));
    assert!(check_deb.contains("usr/share/man/man1/occluview-cli.1.gz"));
    assert!(check_deb.contains("/usr/share/common-licenses/Apache-2.0"));
    assert!(check_deb.contains("desktop-file-validate"));
    assert!(check_deb.contains("appstreamcli validate --no-net"));
    assert!(check_deb.contains("xmllint --noout"));
    assert!(check_deb.contains("lintian --fail-on error"));
    assert!(check_deb.contains("ldd"));
}

#[test]
fn package_pipeline_can_sign_windows_artifacts_when_certificate_is_configured() {
    let build_msi = include_str!("../../../install/build-msi.ps1");

    assert!(build_msi.contains("[ValidateSet(\"auto\", \"none\", \"certstore\", \"pfx\")]"));
    assert!(build_msi.contains("Find-SignTool"));
    assert!(build_msi.contains("Resolve-SigningMode"));
    assert!(build_msi.contains("Sign-WindowsArtifact"));
    assert!(build_msi.contains("signtool.exe"));
    assert!(build_msi.contains("Get-AuthenticodeSignature"));
    assert!(build_msi.contains("OCCLUVIEW_SIGN_CERT_SHA1"));
    assert!(build_msi.contains("OCCLUVIEW_SIGN_PFX_PATH"));
    assert!(build_msi.contains("OCCLUVIEW_SIGN_PFX_PASSWORD"));
    assert!(build_msi.contains("OCCLUVIEW_SIGN_TIMESTAMP_URL"));
    assert!(build_msi.contains("Signing disabled"));
    assert!(build_msi.contains("occluview.exe"));
    assert!(build_msi.contains("occluview_shell.dll"));
    assert!(!build_msi.contains("\"-p\", \"occluview-cli\""));
    assert!(!build_msi.contains("Join-Path $buildDir \"occluview-cli.exe\""));
    assert!(build_msi.contains("Sign-WindowsArtifact -Path $msiPath"));
    assert!(build_msi.contains("CARGO_ENCODED_RUSTFLAGS"));
    assert!(build_msi.contains("--remap-path-prefix=$repoRoot=occluview"));
    assert!(build_msi.contains("OCCLUVIEW_HPS_EMBEDDED_KEY"));
    assert!(build_msi.contains("occluview-formats/private-hps-key"));
    // The Explorer shell DLL — not just the app — must embed the key, or HPS
    // thumbnails and the preview pane fall back to the placeholder cube while the
    // app opens the same encrypted scan fine.
    assert!(
        build_msi.contains(
            "$shellCargoArgs += @(\"--features\", \"occluview-formats/private-hps-key\")"
        ),
        "the shell build must also enable the private HPS key feature"
    );
    assert!(build_msi.contains("$msiPath"));

    let workspace = include_str!("../../../Cargo.toml");
    assert!(workspace.contains("strip = \"symbols\""));
    assert!(workspace.contains("debug = false"));
    assert!(workspace.contains("panic = \"abort\""));
    // The shell DLL ships from the unwind profile: a panic=abort cdylib takes
    // Explorer's whole dllhost down (every thumbnail in the folder blanks);
    // unwinding lets the COM boundary substitute a placeholder instead.
    assert!(workspace.contains("[profile.release-unwind]"));
    assert!(workspace.contains("panic = \"unwind\""));
    let build_msi_script = include_str!("../../../install/build-msi.ps1");
    assert!(build_msi_script.contains(r#""--profile", "release-unwind""#));

    let workflow = include_str!("../../../.github/workflows/package-msi.yml");
    assert!(workflow.contains("OCCLUVIEW_SIGN_PFX_BASE64"));
    assert!(workflow.contains("OCCLUVIEW_SIGN_PFX_PATH"));
    assert!(workflow.contains("OCCLUVIEW_SIGN_PFX_PASSWORD"));
    assert!(workflow.contains("OCCLUVIEW_SIGN_CERT_SHA1"));
    assert!(workflow.contains("OCCLUVIEW_HPS_EMBEDDED_KEY"));
    assert!(workflow.contains("cargo test --locked -p occluview-hps --features private-hps-key"));
    assert!(workflow.contains("-SignMode auto"));
    assert!(!workflow.contains("OCCLUVIEW_SIGN_PFX_PASSWORD: \""));

    let build_windows = include_str!("../../../scripts/build-windows-msvc.sh");
    assert!(build_windows.contains("OCCLUVIEW_HPS_EMBEDDED_KEY"));
    assert!(build_windows.contains("occluview-formats/private-hps-key"));
    assert!(build_windows.contains("CARGO_ENCODED_RUSTFLAGS"));
    assert!(build_windows.contains("--remap-path-prefix=$repo_root=occluview"));
}

#[test]
fn release_notes_put_the_recommended_installer_before_technical_verification() {
    let workflow = include_str!("../../../.github/workflows/package-msi.yml");
    let notes = workflow
        .split_once("OccluView $RELEASE_TAG")
        .and_then(|(_, rest)| rest.split_once("NOTES"))
        .map(|(notes, _)| notes)
        .expect("release-notes template");

    let download = notes
        .find("## Start here")
        .expect("release notes must start with the download choice");
    let installer = notes
        .find("Windows installer — recommended")
        .expect("release notes must identify the Windows installer as recommended");
    let technical = notes
        .find("<summary>For IT and verification</summary>")
        .expect("technical verification must remain available without leading the release");
    assert!(download < installer && installer < technical);
}

#[test]
fn public_release_uses_plain_package_names() {
    let workflow = include_str!("../../../.github/workflows/package-msi.yml");
    let signing = workflow
        .split_once("- name: Sign update artifacts and write latest.json")
        .and_then(|(_, rest)| {
            rest.split_once("- name: Verify the signatures against the key the updater ships")
        })
        .map(|(signing, _)| signing)
        .expect("release workflow must normalize the customer-facing package names");
    let (_, publish) = workflow
        .split_once("- name: Publish GitHub Release")
        .expect("release workflow must publish the customer-facing downloads");

    for asset in [
        "OccluView-Windows-Setup.msi",
        "OccluView-Windows-Portable.zip",
        "OccluView-Linux.deb",
    ] {
        assert!(
            signing.contains(asset),
            "the signed release package should use the plain name {asset}"
        );
        assert!(
            publish.contains(asset),
            "the customer-facing release should publish {asset}"
        );
    }
}

#[test]
fn public_release_groups_technical_verification_material_into_one_download() {
    let workflow = include_str!("../../../.github/workflows/package-msi.yml");
    let (_, bundle) = workflow
        .split_once("- name: Bundle verification material")
        .expect("release workflow must bundle technical verification material");
    let (_, publish) = workflow
        .split_once("- name: Publish GitHub Release")
        .expect("release workflow must publish the customer-facing downloads");

    assert!(bundle.contains("OccluView-${version}-verification.zip"));
    assert!(bundle.contains("latest.json"));
    assert!(bundle.contains("sbom-*.json"));
    assert!(publish.contains("-name '*-verification.zip'"));
    assert!(publish.contains("-name 'latest.json'"));
    assert!(publish.contains("-name 'latest.json.minisig'"));
    assert!(!publish.contains("-o -name '*.sha256'"));
    assert!(!publish.contains("-o -name '*.minisig'"));
    assert!(!publish.contains("-o -name '*.json'"));
}

#[test]
fn release_note_markdown_is_not_executed_by_the_shell_heredoc() {
    let workflow = include_str!("../../../.github/workflows/package-msi.yml");
    let (_, write_notes) = workflow
        .split_once("- name: Write release notes")
        .expect("release workflow must write customer-facing notes");

    assert!(write_notes.contains("\\`OccluView-Windows-Setup.msi\\`"));
    assert!(write_notes.contains("\\`OccluView-Windows-Portable.zip\\`"));
    assert!(write_notes.contains("\\`OccluView-Linux.deb\\`"));
    assert!(write_notes.contains("\\`OccluView-${RELEASE_TAG#v}-verification.zip\\`"));
}

#[test]
fn release_version_is_kept_in_sync_across_workspace_lockfile_and_installer() {
    let cargo_toml = include_str!("../../../Cargo.toml");
    let cargo_lock = include_str!("../../../Cargo.lock");
    let wxs = include_str!("../../../install/occluview.wxs");

    let version = workspace_package_version(cargo_toml);
    assert!(version.is_some(), "workspace package version is present");
    let Some(version) = version else {
        return;
    };
    let wix_version = wix_product_version(wxs);
    assert!(
        wix_version.is_some(),
        "WiX fallback product version is present"
    );
    let Some(wix_version) = wix_version else {
        return;
    };
    assert_eq!(
        wix_version, version,
        "WiX ProductVersion fallback must match Cargo workspace version"
    );

    for package in [
        "occluview-app",
        "occluview-cli",
        "occluview-core",
        "occluview-formats",
        "occluview-render",
        "occluview-shell",
        "occluview-thumbnail",
        "occluview-update",
        "occluview-hps",
        "occluview-robust-csg",
        "occlu-mesh-edit",
    ] {
        assert_eq!(
            cargo_lock_package_version(cargo_lock, package),
            Some(version),
            "{package} version in Cargo.lock must match Cargo workspace version"
        );
    }
}

#[test]
fn gui_file_association_launches_reuse_existing_window() {
    let app_main = include_str!("../../occluview-app/src/main.rs");
    let bootstrap = include_str!("../../occluview-app/src/app_bootstrap.rs");
    let app_loading = include_str!("../../occluview-app/src/app/app_loading.rs");
    let app_state = include_str!("../../occluview-app/src/app/state.rs");
    let single_instance = include_str!("../../occluview-app/src/single_instance/mod.rs");
    let single_instance_windows =
        include_str!("../../occluview-app/src/single_instance/windows.rs");

    assert!(app_main.contains("mod single_instance"));
    assert!(bootstrap.contains("SingleInstance::acquire"));
    assert!(bootstrap.contains("write_open_request(&request)"));
    assert!(app_state.contains("incoming_open_requests: single_instance::OpenRequestListener"));
    assert!(app_loading.contains("fn open_paths_from_external_source("));
    assert!(app_loading.contains("for request in self.incoming_open_requests.take_requests()"));
    assert!(app_loading
        .contains("self.open_paths_from_external_source(&request.paths, \"single-instance\")"));
    assert!(single_instance_windows.contains("CreateMutexW"));
    assert!(single_instance_windows.contains("Local\\\\OccluTrace.OccluView.SingleInstance"));
    assert!(single_instance_windows.contains("CreateNamedPipeW"));
    assert!(single_instance_windows.contains("WaitNamedPipeW"));
    assert!(single_instance.contains("open-requests"));
}

#[test]
fn self_registration_unregister_only_removes_occluview_values() {
    let registration = registration_source();

    assert!(registration.contains("GetModuleHandleExW"));
    assert!(registration.contains("GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS"));
    assert!(!registration.contains("GetModuleFileNameW(None"));
    assert!(registration.contains("fn format_progid(ext: &str)"));
    assert!(registration.contains("MeshFile."));
    assert!(registration.contains("format_file_type_name"));
    assert!(registration.contains("occluview-3d.ico"));
    assert!(registration.contains("ThumbnailCutoff"));
    assert!(registration.contains("TypeOverlay"));
    assert!(registration.contains("LEGACY_OCCLUVIEW_PROGID"));
    assert!(registration.contains("fn format_legacy_progid(ext: &str)"));
    assert!(registration.contains("delete_value(&key_path, Some(&legacy_format))"));
    assert!(!registration.contains("pub const OCCLUVIEW_PROGID"));
    assert!(registration.contains("RegDeleteValueW"));
    assert!(registration.contains("delete_value_if_matches(&key_path, None, &our_clsid)"));
    assert!(registration.contains("delete_value(&key_path, Some(&progid))"));
    assert!(registration.contains("is_occluview_default_icon_value"));
    assert!(!registration.contains("delete_tree(&key_path)"));
}

#[test]
fn com_thumbnail_provider_accepts_file_paths_for_extension_hints() {
    let com = [
        include_str!("com.rs"),
        include_str!("com/thumbnail_provider.rs"),
        include_str!("com/preview.rs"),
        include_str!("com/preview/theme.rs"),
        include_str!("com/preview/window.rs"),
    ]
    .join("\n");
    let smoke = include_str!("../../../install/test-thumbnail-provider.ps1");

    assert!(com.contains("IInitializeWithFile"));
    assert!(com.contains("IInitializeWithItem"));
    assert!(com.contains("impl IInitializeWithFile_Impl for ThumbnailProvider_Impl"));
    assert!(com.contains("impl IInitializeWithItem_Impl for ThumbnailProvider_Impl"));
    assert!(com.contains("try_render_thumbnail_file(&path, spec, DEFAULT_THUMBNAIL_TIMEOUT)"));
    assert!(!com.contains("ThumbnailProvider::read_file(&path)"));
    assert!(!com.contains("std::fs::read(path)"));
    assert!(com.contains(".initialize_path(path.clone(), path_extension(&path));"));
    assert!(com.contains("fn path_extension(path: &Path) -> Option<String>"));
    assert!(smoke.contains("public interface IInitializeWithItem"));
    assert!(smoke.contains("public interface IShellItem"));
    assert!(smoke.contains("ProbeDirectFromItem"));
    assert!(smoke.contains("SHCreateShellItemFromParsingName"));
}

fn workspace_package_version(cargo_toml: &str) -> Option<&str> {
    let section = cargo_toml.split("[workspace.package]").nth(1)?;
    toml_quoted_value(section, "version")
}

fn cargo_lock_package_version<'a>(cargo_lock: &'a str, package_name: &str) -> Option<&'a str> {
    let package_line = format!("name = \"{package_name}\"");
    cargo_lock
        .split("[[package]]")
        .find(|block| block.lines().any(|line| line.trim() == package_line))
        .and_then(|block| toml_quoted_value(block, "version"))
}

fn wix_product_version(wxs: &str) -> Option<&str> {
    let marker = "<?define ProductVersion = \"";
    let rest = wxs.get(wxs.find(marker)? + marker.len()..)?;
    rest.get(..rest.find('"')?)
}

fn toml_quoted_value<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            continue;
        }
        let Some(rest) = trimmed.strip_prefix(key) else {
            continue;
        };
        let rest = rest.trim_start();
        let Some(rest) = rest.strip_prefix('=') else {
            continue;
        };
        let rest = rest.trim_start();
        let Some(rest) = rest.strip_prefix('"') else {
            continue;
        };
        return rest.get(..rest.find('"')?);
    }
    None
}

#[test]
fn explorer_preview_never_activates_a_hardware_renderer() {
    let factory = include_str!("offscreen_factory.rs");

    assert!(
        !factory.contains("Offscreen::new_prefer_hardware()"),
        "the preview handler runs in Prevhost; it must not invoke a workstation GPU driver"
    );
    assert!(factory.contains("Offscreen::new()"));
}

#[test]
fn explorer_thumbnail_host_forces_the_software_renderer_before_prewarm() {
    let com = include_str!("com.rs");

    let thumbnail_start = com
        .find("if *class == OCCLUVIEW_THUMBNAIL_GUID {")
        .expect("thumbnail prewarm branch");
    let thumbnail_end = com[thumbnail_start..]
        .find("\n}\n\nfn path_extension")
        .map(|offset| thumbnail_start + offset)
        .expect("thumbnail prewarm branch end");
    let thumbnail_branch = &com[thumbnail_start..thumbnail_end];
    assert!(
        thumbnail_branch.contains("occluview_thumbnail::use_software_renderer_only();"),
        "the out-of-process thumbnail host must choose software before it starts its renderer"
    );
}
