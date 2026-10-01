#![allow(clippy::expect_used)]

use super::*;
use roxmltree::{Document, Node};
use std::collections::BTreeSet;

const MACOS_CONTENT_TYPES: [(&str, &str); 7] = [
    ("stl", "public.standard-tesselated-geometry-format"),
    ("ply", "public.polygon-file-format"),
    ("obj", "public.geometry-definition-format"),
    ("glb", "org.khronos.glb"),
    ("off", "ai.occlutrace.occluview.off"),
    ("hps", "ai.occlutrace.occluview.hps"),
    ("dcm", "org.nema.dicom"),
];

fn plist_value<'a, 'input>(dictionary: Node<'a, 'input>, key: &str) -> Option<Node<'a, 'input>> {
    let mut children = dictionary.children().filter(Node::is_element);
    while let Some(candidate) = children.next() {
        let value = children.next()?;
        if candidate.tag_name().name() == "key" && candidate.text() == Some(key) {
            return Some(value);
        }
    }
    None
}

fn array_has_string(array: Node<'_, '_>, expected: &str) -> bool {
    array.children().any(|item| {
        item.is_element() && item.tag_name().name() == "string" && item.text() == Some(expected)
    })
}

fn plist_dictionary<'a, 'input>(plist: &'a Document<'input>) -> Node<'a, 'input> {
    let root = plist.root_element();
    assert_eq!(root.tag_name().name(), "plist");
    root.children()
        .find(|node| node.is_element() && node.tag_name().name() == "dict")
        .expect("the property list root contains a dictionary")
}

fn plist_document_type<'a, 'input>(
    document_types: Node<'a, 'input>,
    identifier: &str,
) -> Option<Node<'a, 'input>> {
    document_types
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == "dict")
        .find(|entry| {
            plist_value(*entry, "LSItemContentTypes")
                .is_some_and(|array| array_has_string(array, identifier))
        })
}

#[cfg(windows)]
#[test]
fn windows_process_and_start_menu_shortcut_share_the_app_identity() {
    let source = repo_file("../../install/occluview.wxs");
    let document = Document::parse(&source).expect("WiX source is well-formed XML");
    let shortcut = document.descendants().find(|node| {
        node.is_element()
            && node.tag_name().name() == "ShortcutProperty"
            && node.attribute("Key") == Some("System.AppUserModel.ID")
    });

    assert_eq!(
        shortcut.and_then(|node| node.attribute("Value")),
        Some(APP_USER_MODEL_ID),
        "the installed shortcut must use the identity assigned to the process"
    );
}

#[test]
fn macos_document_types_are_well_formed_and_match_the_open_formats() {
    let source = repo_file("../../install/macos/Info.plist.in");
    let plist = Document::parse_with_options(
        &source,
        roxmltree::ParsingOptions {
            allow_dtd: true,
            ..roxmltree::ParsingOptions::default()
        },
    )
    .expect("Info.plist is well-formed XML");
    let root = plist_dictionary(&plist);

    assert_eq!(
        plist_value(root, "CFBundleExecutable").and_then(|node| node.text()),
        Some("occluview")
    );
    assert_eq!(
        plist_value(root, "LSMinimumSystemVersion").and_then(|node| node.text()),
        Some("14.0")
    );

    let document_types = plist_value(root, "CFBundleDocumentTypes")
        .filter(|node| node.tag_name().name() == "array")
        .expect("the bundle declares document types");
    let entries: Vec<_> = document_types
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == "dict")
        .collect();
    assert!(
        !entries.is_empty(),
        "Finder has at least one supported type"
    );
    assert_document_type_ranks(document_types, &entries);
    assert_document_type_mappings(document_types, &entries);
    assert_hps_imported_extension(root);
}

fn assert_document_type_ranks(document_types: Node<'_, '_>, entries: &[Node<'_, '_>]) {
    for entry in entries {
        assert_eq!(
            plist_value(*entry, "CFBundleTypeRole").and_then(|node| node.text()),
            Some("Viewer"),
            "the bundle opens supported formats for viewing"
        );
        assert_ne!(
            plist_value(*entry, "LSHandlerRank").and_then(|node| node.text()),
            Some("Owner"),
            "the viewer does not claim an input format as its system owner"
        );
    }

    let hps = plist_document_type(document_types, "ai.occlutrace.occluview.hps")
        .expect("the HPS type is offered to Finder");
    assert_eq!(
        plist_value(hps, "LSHandlerRank").and_then(|node| node.text()),
        Some("Default")
    );

    let dicom = plist_document_type(document_types, "org.nema.dicom")
        .expect("the .dcm type is offered to Finder");
    assert_eq!(
        plist_value(dicom, "LSHandlerRank").and_then(|node| node.text()),
        Some("Alternate"),
        "medical DICOM files must remain opt-in"
    );
}

fn assert_document_type_mappings(document_types: Node<'_, '_>, entries: &[Node<'_, '_>]) {
    let declared_types: Vec<String> = entries
        .iter()
        .flat_map(|entry| {
            plist_value(*entry, "LSItemContentTypes")
                .filter(|node| node.tag_name().name() == "array")
                .expect("each Finder document type declares its UTIs")
                .children()
                .filter(|node| node.is_element() && node.tag_name().name() == "string")
                .map(|node| node.text().expect("UTI entries contain strings").to_owned())
        })
        .collect();
    let expected_types: BTreeSet<String> = MACOS_CONTENT_TYPES
        .iter()
        .map(|(_, identifier)| (*identifier).to_owned())
        .collect();
    let declared_extensions: BTreeSet<&str> = MACOS_CONTENT_TYPES
        .iter()
        .map(|(extension, _)| *extension)
        .collect();
    let supported_extensions: BTreeSet<&str> = occluview_formats::V1_OPEN_EXTENSIONS
        .iter()
        .copied()
        .collect();
    assert_eq!(
        declared_extensions.len(),
        MACOS_CONTENT_TYPES.len(),
        "the plist mapping has no duplicate extensions"
    );
    assert_eq!(
        supported_extensions.len(),
        occluview_formats::V1_OPEN_EXTENSIONS.len(),
        "the format reader lists each extension once"
    );
    assert_eq!(
        declared_extensions, supported_extensions,
        "Finder registers exactly the extensions the viewer opens"
    );
    assert_eq!(
        declared_types.len(),
        expected_types.len(),
        "Finder declares each supported UTI exactly once"
    );
    assert_eq!(
        declared_types.iter().cloned().collect::<BTreeSet<_>>(),
        expected_types,
        "Finder registers exactly the formats the viewer can open"
    );

    for &(extension, identifier) in &MACOS_CONTENT_TYPES {
        assert!(
            occluview_formats::V1_OPEN_EXTENSIONS.contains(&extension),
            "the viewer accepts .{extension}"
        );
        assert!(
            occluview_formats::probe::by_extension(extension).is_some(),
            "the format reader accepts .{extension}"
        );
        assert!(
            plist_document_type(document_types, identifier).is_some(),
            "Finder must offer the app for .{extension} through {identifier}"
        );
    }
}

fn assert_hps_imported_extension(root: Node<'_, '_>) {
    let imported_types = plist_value(root, "UTImportedTypeDeclarations")
        .filter(|node| node.tag_name().name() == "array")
        .expect("the bundle imports the private HPS content type");
    let hps_declaration = imported_types
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == "dict")
        .find(|entry| {
            plist_value(*entry, "UTTypeIdentifier").and_then(|node| node.text())
                == Some("ai.occlutrace.occluview.hps")
        })
        .expect("the HPS content type has an imported declaration");
    let tag_specification = plist_value(hps_declaration, "UTTypeTagSpecification")
        .expect("the HPS type declares its filename extension");
    let extensions = plist_value(tag_specification, "public.filename-extension")
        .filter(|node| node.tag_name().name() == "array")
        .expect("the HPS type has a filename extension array");
    let hps_extensions: BTreeSet<String> = extensions
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == "string")
        .map(|node| {
            node.text()
                .expect("extension entries contain strings")
                .to_owned()
        })
        .collect();
    assert_eq!(hps_extensions, BTreeSet::from(["hps".to_owned()]));
    assert!(plist_value(root, "UTExportedTypeDeclarations").is_none());
}

#[cfg(target_os = "macos")]
#[test]
fn system_uniform_type_identifiers_resolve_each_supported_extension() {
    let extensions: Vec<&str> = MACOS_CONTENT_TYPES
        .iter()
        .map(|(extension, _)| *extension)
        .filter(|extension| *extension != "hps")
        .collect();
    let source = r#"
import Foundation
import UniformTypeIdentifiers

let extensions = ProcessInfo.processInfo.environment["OCCLUVIEW_TEST_EXTENSIONS"]!
for ext in extensions.split(separator: ",") {
    if let type = UTType(filenameExtension: String(ext)) {
        print("\(ext)=\(type.identifier)")
    } else {
        print("\(ext)=")
    }
}
"#;
    let output = std::process::Command::new("swift")
        .args(["-e", source])
        .env("OCCLUVIEW_TEST_EXTENSIONS", extensions.join(","))
        .output()
        .expect("the macOS runner can start Swift with UniformTypeIdentifiers");
    assert!(
        output.status.success(),
        "Uniform Type lookup failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let resolved: std::collections::BTreeMap<_, _> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(extension, identifier)| (extension.to_owned(), identifier.to_owned()))
        .collect();
    for (extension, expected) in MACOS_CONTENT_TYPES
        .iter()
        .filter(|(extension, _)| *extension != "hps")
    {
        assert_eq!(
            resolved.get(*extension).map(String::as_str),
            Some(*expected),
            "Uniform Type lookup for .{extension} matches the Finder declaration"
        );
    }
}

#[test]
fn native_dependency_notices_name_the_shipped_components_and_licenses() {
    let notices = include_str!("../../../../THIRD-PARTY-NOTICES-NATIVE.md");
    for component in ["Manifold", "oneTBB", "Clipper2"] {
        assert!(
            notices.contains(component),
            "notice text must name {component}"
        );
    }
    assert!(notices.contains("Apache License"));
    assert!(notices.contains("Boost Software License"));
    assert!(notices.contains("tag, not a commit"));
    assert!(notices.contains("cargo deny") && notices.contains("SBOM"));
}
