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
}

#[test]
fn the_msi_tag_check_is_a_boolean_and_not_a_command_call() {
    // `Test-HasText $env:GITHUB_REF -and (...)` parses in PowerShell's command
    // mode: the function is called with three positional arguments, declares
    // one, and drops the rest. The expression then means "GITHUB_REF is set",
    // so every CI run counts as a tagged release and demands a signing
    // certificate -- which bites precisely where there is none, in a fork or in
    // the packaging rehearsal before secrets exist.
    let script = include_str!("../../../install/build-msi.ps1");
    assert!(
        script.contains(
            "(Test-HasText $env:GITHUB_REF) -and ($env:GITHUB_REF -like \"refs/tags/v*\")"
        ),
        "the tagged-release test must be a parenthesised boolean expression"
    );
    assert!(
        !script.contains("= Test-HasText $env:GITHUB_REF -and"),
        "the unparenthesised form is a command call with discarded arguments"
    );
}
