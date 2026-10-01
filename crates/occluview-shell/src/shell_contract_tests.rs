use super::{
    owns_extension, APP_EXE_NAME, DEDICATED_FILE_ICON_EXTENSIONS, OFFERED_ONLY_EXTENSIONS,
    PREVIEW_HANDLER_CATEGORY, SUPPORTED_EXTENSIONS, THUMBNAIL_PROVIDER_CATEGORY,
};
use roxmltree::{Document, Node};
use std::collections::BTreeSet;
use std::path::PathBuf;

fn repo_file(path: &str) -> String {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    std::fs::read_to_string(root.join(path)).expect("repository contract input is readable")
}

fn registry_key<'a, 'input>(document: &'a Document<'input>, key: &str) -> Option<Node<'a, 'input>> {
    document.descendants().find(|node| {
        node.is_element()
            && node.tag_name().name() == "RegistryKey"
            && node
                .attribute("Key")
                .is_some_and(|value| expand_wix_value(document, value) == key)
    })
}

fn expand_wix_value(document: &Document<'_>, value: &str) -> String {
    let mut expanded = value.to_owned();
    for instruction in document.descendants().filter_map(|node| node.pi()) {
        let Some(definition) = instruction.value.filter(|_| instruction.target == "define") else {
            continue;
        };
        let Some((name, value)) = definition.split_once('=') else {
            continue;
        };
        let name = name.trim();
        let value = value.trim().trim_matches('"');
        expanded = expanded.replace(&format!("$(var.{name})"), value);
    }
    expanded
}

fn registry_value(document: &Document<'_>, key: &str, name: Option<&str>) -> Option<String> {
    let key = registry_key(document, key)?;
    key.children()
        .filter(|node| node.is_element() && node.tag_name().name() == "RegistryValue")
        .find(|node| node.attribute("Name") == name)
        .and_then(|node| node.attribute("Value"))
        .map(str::to_owned)
}

fn wix_element_has_id(document: &Document<'_>, element: &str, id: &str) -> bool {
    document.descendants().any(|node| {
        node.is_element() && node.tag_name().name() == element && node.attribute("Id") == Some(id)
    })
}

fn registry_section_exists(registry: &str, key: &str) -> bool {
    let expected = format!("[{key}]");
    registry.lines().map(str::trim).any(|line| line == expected)
}

fn registry_string_value(registry: &str, key: &str, name: &str) -> Option<String> {
    let mut in_section = false;
    for line in registry.lines().map(str::trim) {
        if let Some(section) = line
            .strip_prefix('[')
            .and_then(|line| line.strip_suffix(']'))
        {
            in_section = section == key;
            continue;
        }
        if !in_section || line.is_empty() || line.starts_with(';') {
            continue;
        }
        let Some((field, value)) = line.split_once('=') else {
            continue;
        };
        if field.trim().trim_matches('"') == name {
            return Some(value.trim().trim_matches('"').to_owned());
        }
    }
    None
}

fn progid_extension(extension: &str) -> String {
    if extension == "dcm" {
        "HPS".to_owned()
    } else {
        extension.to_ascii_uppercase()
    }
}

fn assert_supported_registry_entry(wix: &Document<'_>, extension: &str) {
    assert!(
        occluview_formats::probe::by_extension(extension).is_some(),
        "Explorer must not advertise an extension without a reader: {extension}"
    );
    let class = progid_extension(extension);
    let progid = format!("MeshFile.{class}");
    let progid_key = format!("Software\\Classes\\{progid}");
    let open_with_key = format!("Software\\Classes\\.{extension}\\OpenWithProgids");
    assert_eq!(
        registry_value(wix, &open_with_key, Some(progid.as_str())).as_deref(),
        Some(""),
        "the MSI Open with entry must resolve to {progid}"
    );
    assert_eq!(
        registry_value(wix, &progid_key, None).as_deref(),
        Some(format!("{class} File").as_str()),
        "the registered file type name must describe {extension}"
    );

    for category in [THUMBNAIL_PROVIDER_CATEGORY, PREVIEW_HANDLER_CATEGORY] {
        let key = format!("{progid_key}\\ShellEx\\{category}");
        assert!(registry_key(wix, &key).is_some());
    }
    let dot_extension_key = format!("Software\\Classes\\.{extension}");
    if owns_extension(extension) {
        assert_eq!(
            registry_value(wix, &dot_extension_key, None).as_deref(),
            Some(progid.as_str()),
            "the owned extension must resolve to its Open with ProgID"
        );
        for category in [THUMBNAIL_PROVIDER_CATEGORY, PREVIEW_HANDLER_CATEGORY] {
            let direct = format!("{dot_extension_key}\\ShellEx\\{category}");
            let system = format!(
                "Software\\Classes\\SystemFileAssociations\\.{extension}\\ShellEx\\{category}"
            );
            assert!(registry_key(wix, &direct).is_some());
            assert!(registry_key(wix, &system).is_some());
        }
    } else {
        assert_eq!(registry_value(wix, &dot_extension_key, None), None);
        assert!(registry_key(wix, &format!("{dot_extension_key}\\DefaultIcon")).is_none());
        for category in [THUMBNAIL_PROVIDER_CATEGORY, PREVIEW_HANDLER_CATEGORY] {
            let direct = format!("{dot_extension_key}\\ShellEx\\{category}");
            let system = format!(
                "Software\\Classes\\SystemFileAssociations\\.{extension}\\ShellEx\\{category}"
            );
            assert!(registry_key(wix, &direct).is_none());
            assert!(registry_key(wix, &system).is_none());
        }
    }

    let edit_key =
        format!("Software\\Classes\\SystemFileAssociations\\.{extension}\\shell\\OccluView.Edit");
    assert!(registry_key(wix, &edit_key).is_some());
    assert!(registry_key(wix, &format!("{edit_key}\\command")).is_some());

    if DEDICATED_FILE_ICON_EXTENSIONS.contains(&extension) {
        let icon_key = format!("{progid_key}\\DefaultIcon");
        assert_eq!(
            registry_value(wix, &icon_key, None).as_deref(),
            Some("[INSTALLFOLDER]occluview-3d.ico")
        );
    }
}

#[test]
fn explorer_open_with_and_icon_entries_match_the_reader_set() {
    let source = repo_file("install/occluview.wxs");
    let wix = Document::parse(&source).expect("WiX source is well-formed XML");
    assert_eq!(
        wix_variable(&wix, "ThumbnailProviderCategory"),
        Some(THUMBNAIL_PROVIDER_CATEGORY.to_owned())
    );
    assert_eq!(
        wix_variable(&wix, "PreviewHandlerCategory"),
        Some(PREVIEW_HANDLER_CATEGORY.to_owned())
    );
    for (component, file) in [
        ("cmpLicenseFile", "filLicenseFile"),
        ("cmpNoticeFile", "filNoticeFile"),
        ("cmpThirdPartyNotices", "filThirdPartyNotices"),
        ("cmpThirdPartyNoticesNative", "filThirdPartyNoticesNative"),
    ] {
        assert!(wix_element_has_id(&wix, "Component", component));
        assert!(wix_element_has_id(&wix, "File", file));
        assert!(wix_element_has_id(&wix, "ComponentRef", component));
    }
    let major_upgrade = wix
        .descendants()
        .find(|node| node.is_element() && node.tag_name().name() == "MajorUpgrade")
        .expect("the MSI declares its upgrade policy");
    assert_eq!(
        major_upgrade.attribute("Schedule"),
        Some("afterInstallInitialize")
    );
    assert_ne!(
        major_upgrade.attribute("AllowSameVersionUpgrades"),
        Some("yes")
    );

    for extension in SUPPORTED_EXTENSIONS {
        assert_supported_registry_entry(&wix, extension);
    }

    assert_eq!(
        OFFERED_ONLY_EXTENSIONS,
        [occluview_formats::LEGACY_HPS_EXTENSION]
    );
    assert!(!owns_extension("dcm"));
    for extension in ["stl", "ply", "obj", "glb", "hps"] {
        assert!(owns_extension(extension));
    }
    for forbidden in [
        "Software\\Classes\\.dcm",
        "Software\\Classes\\.dcm\\DefaultIcon",
        "Software\\Classes\\.dcm\\ShellEx",
        "Software\\Classes\\SystemFileAssociations\\.dcm\\ShellEx",
    ] {
        assert!(
            registry_key(&wix, forbidden).is_none(),
            "the installer must not claim medical DICOM files through {forbidden}"
        );
    }
    assert!(registry_key(
        &wix,
        "Software\\Classes\\SystemFileAssociations\\.dcm\\shell\\OccluView.Edit"
    )
    .is_some());
    assert!(
        registry_key(
            &wix,
            &format!("Software\\Classes\\Applications\\{APP_EXE_NAME}")
        )
        .is_some(),
        "the Open with executable entry must name the shipped viewer"
    );

    assert_manual_registry_contract(&wix);
}

fn assert_manual_registry_contract(wix: &Document<'_>) {
    let manual = repo_file("install/occluview-shell-registration.reg");
    let preview_app_id = "{6D2B5079-2F0B-48DD-AB7F-97CEC514D30B}";
    assert_eq!(
        wix_variable(wix, "PrevhostAppId").as_deref(),
        Some(preview_app_id)
    );
    assert_eq!(
        registry_string_value(
            &manual,
            "HKEY_CLASSES_ROOT\\CLSID\\{9F3A1B2C-4D5E-4F60-8A7B-9C0D1E2F3046}",
            "AppID"
        )
        .as_deref(),
        Some(preview_app_id),
        "manual shell registration uses Windows' Prevhost AppID"
    );
    assert!(!registry_section_exists(
        &manual,
        &format!("HKEY_CLASSES_ROOT\\AppID\\{preview_app_id}")
    ));
    for extension in SUPPORTED_EXTENSIONS {
        let dot_extension = format!("HKEY_CLASSES_ROOT\\.{extension}");
        if owns_extension(extension) {
            assert_eq!(
                registry_string_value(&manual, &dot_extension, "@"),
                Some(format!("MeshFile.{}", progid_extension(extension)))
            );
        } else {
            assert_eq!(registry_string_value(&manual, &dot_extension, "@"), None);
            assert!(!registry_section_exists(
                &manual,
                &format!("{dot_extension}\\DefaultIcon")
            ));
            for category in [THUMBNAIL_PROVIDER_CATEGORY, PREVIEW_HANDLER_CATEGORY] {
                assert!(!registry_section_exists(
                    &manual,
                    &format!("{dot_extension}\\ShellEx\\{category}")
                ));
                assert!(!registry_section_exists(
                    &manual,
                    &format!(
                        "HKEY_CLASSES_ROOT\\SystemFileAssociations\\.{extension}\\ShellEx\\{category}"
                    )
                ));
            }
        }
    }
    assert_eq!(
        registry_string_value(
            &manual,
            "HKEY_CLASSES_ROOT\\.dcm\\OpenWithProgids",
            "MeshFile.HPS"
        ),
        Some(String::new()),
        ".dcm remains available through the explicit Open With menu"
    );
    assert_eq!(
        registry_string_value(
            &manual,
            "HKEY_CLASSES_ROOT\\Applications\\occluview.exe\\SupportedTypes",
            ".dcm"
        ),
        Some(String::new()),
        ".dcm remains visible in Windows app discovery"
    );
}

/// The installer's icon entries and the crate's declared list are two independent
/// facts, so they can disagree — which is the assertion's whole value. While
/// `DEDICATED_FILE_ICON_EXTENSIONS` aliased `SUPPORTED_EXTENSIONS` it could not
/// fail.
#[test]
fn the_shipped_icon_entries_match_the_declared_icon_list() {
    let source = repo_file("install/occluview.wxs");
    let wix = Document::parse(&source).expect("WiX source is well-formed XML");
    let shipped: BTreeSet<String> = wix
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "RegistryKey")
        .filter_map(|node| node.attribute("Key"))
        .filter_map(|key| key.strip_prefix("Software\\Classes\\MeshFile."))
        .filter_map(|class| class.strip_suffix("\\DefaultIcon"))
        .map(|class| {
            // HPS is the one class whose extension does not match its ProgID
            // spelling: `.hps` and the legacy `.dcm` share it.
            if class == "HPS" {
                "hps".to_owned()
            } else {
                class.to_ascii_lowercase()
            }
        })
        .collect();
    let declared: BTreeSet<String> = DEDICATED_FILE_ICON_EXTENSIONS
        .iter()
        .map(|extension| (*extension).to_owned())
        .collect();
    assert_eq!(
        declared.len(),
        DEDICATED_FILE_ICON_EXTENSIONS.len(),
        "the declared icon list repeats an extension"
    );
    assert_eq!(
        shipped, declared,
        "the installer ships a DefaultIcon for exactly the declared extensions"
    );
}

fn wix_variable(document: &Document<'_>, name: &str) -> Option<String> {
    document
        .descendants()
        .filter_map(|node| node.pi())
        .filter(|instruction| instruction.target == "define")
        .filter_map(|instruction| instruction.value)
        .filter_map(|definition| definition.split_once('='))
        .find_map(|(defined_name, value)| {
            (defined_name.trim() == name).then(|| value.trim().trim_matches('"').to_owned())
        })
}

#[test]
fn linux_mime_metadata_distinguishes_hps_dcm_from_medical_dicom() {
    let source = repo_file("install/linux/occluview-mime.xml");
    let mime = Document::parse(&source).expect("shared MIME metadata is well-formed XML");
    let hps = mime
        .descendants()
        .find(|node| {
            node.is_element()
                && node.tag_name().name() == "mime-type"
                && node.attribute("type") == Some("application/x-occluview-hps")
        })
        .expect("the HPS MIME type is declared");

    for pattern in ["*.dcm", "*.DCM"] {
        assert!(hps.children().any(|node| {
            node.is_element()
                && node.tag_name().name() == "glob"
                && node.attribute("pattern") == Some(pattern)
                && node.attribute("weight") == Some("40")
        }));
    }
    for pattern in ["*.hps", "*.HPS"] {
        assert!(hps.children().any(|node| {
            node.is_element()
                && node.tag_name().name() == "glob"
                && node.attribute("pattern") == Some(pattern)
        }));
    }
    assert!(hps.descendants().any(|node| {
        node.is_element()
            && node.tag_name().name() == "magic"
            && node.attribute("priority") == Some("60")
            && node.children().any(|matcher| {
                matcher.is_element()
                    && matcher.tag_name().name() == "match"
                    && matcher.attribute("value") == Some("<HPS")
                    && matcher.attribute("offset") == Some("0:256")
            })
    }));

    let wavefront = mime
        .descendants()
        .find(|node| {
            node.is_element()
                && node.tag_name().name() == "mime-type"
                && node.attribute("type") == Some("model/obj")
        })
        .expect("the Wavefront MIME type is declared");
    for pattern in ["*.obj", "*.OBJ"] {
        assert!(wavefront.children().any(|node| {
            node.is_element()
                && node.tag_name().name() == "glob"
                && node.attribute("pattern") == Some(pattern)
                && node.attribute("weight") == Some("60")
        }));
    }

    let ply = mime
        .descendants()
        .find(|node| {
            node.is_element()
                && node.tag_name().name() == "mime-type"
                && node.attribute("type") == Some("application/x-ply")
        })
        .expect("the PLY MIME type is declared");
    for pattern in ["*.ply", "*.PLY"] {
        assert!(ply.children().any(|node| {
            node.is_element()
                && node.tag_name().name() == "glob"
                && node.attribute("pattern") == Some(pattern)
        }));
    }
}

#[test]
fn diagnostic_events_keep_a_fixed_privacy_safe_shape() {
    use crate::shell_diagnostics::{
        ShellDiagnosticAdapter, ShellDiagnosticComponent, ShellDiagnosticEvent,
        ShellDiagnosticEventInput, ShellDiagnosticOutcome, ShellDiagnosticProcess,
        ShellDiagnosticStage,
    };

    let event = ShellDiagnosticEvent::normal(
        ShellDiagnosticEventInput {
            component: ShellDiagnosticComponent::Preview,
            stage: ShellDiagnosticStage::BitmapPublish,
            adapter: ShellDiagnosticAdapter::Hardware,
            elapsed_ms: 18,
        },
        ShellDiagnosticOutcome::Completed,
        ShellDiagnosticProcess {
            timestamp_unix_ms: 1_725_000_001,
            process_id: 42,
        },
    )
    .json_line();
    let parsed: serde_json::Value = serde_json::from_str(&event).expect("diagnostic JSON parses");

    for (field, expected) in [
        ("component", "preview"),
        ("stage", "bitmap_publish"),
        ("outcome", "completed"),
        ("adapter", "hardware"),
    ] {
        assert_eq!(parsed[field], expected);
    }
    assert_eq!(parsed["elapsed_ms"], 18);
    for field in ["path", "filename", "driver", "error"] {
        assert!(parsed.get(field).is_none(), "diagnostics omit {field}");
    }
}

#[cfg(windows)]
#[test]
#[allow(clippy::panic)]
fn com_entry_returns_the_fallback_when_the_body_panics() {
    let value = crate::com::com_entry("test::body_returns", || 0_u32, || 7);
    assert_eq!(value, 7, "a successful COM body returns its result");

    let caught = crate::com::com_entry("test::body_panics", || 0_u32, || panic!("boom"));
    assert_eq!(caught, 0, "a COM entry converts a panic into its fallback");
}
