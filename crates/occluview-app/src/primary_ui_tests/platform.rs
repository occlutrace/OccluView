#![allow(clippy::expect_used)]

use super::*;
use roxmltree::{Document, Node};

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
    let plist = Document::parse(&source).expect("Info.plist is well-formed XML");
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
    for entry in &entries {
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

    let content_types = [
        ("stl", "public.standard-tesselated-geometry-format"),
        ("ply", "public.polygon-file-format"),
        ("obj", "public.geometry-definition-format"),
        ("glb", "org.khronos.glb"),
        ("hps", "ai.occlutrace.occluview.hps"),
        ("dcm", "org.nema.dicom"),
    ];
    assert_eq!(
        content_types.len(),
        occluview_formats::V1_OPEN_EXTENSIONS.len()
    );
    for (extension, identifier) in content_types {
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
    assert!(array_has_string(extensions, "hps"));
    assert!(plist_value(root, "UTExportedTypeDeclarations").is_none());
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
