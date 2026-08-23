//! Guards over the documents the build ships with.
//!
//! The changelog against the version being prepared, and the usage guide
//! against the keys the viewer actually binds. Both drift silently: nothing
//! fails to compile when a guide describes a shortcut the build does not have.

use super::*;

#[test]
fn the_changelog_only_names_versions_that_can_be_released() {
    // The release job publishes the section matching the tag and nothing else.
    // With one section per local version bump -- 1.0.7, 1.0.8, 1.0.9, none of
    // them tagged -- a single v1.0.9 release would have silently dropped two
    // versions' worth of changes from its notes, while the changelog advertised
    // three versions nobody could download.
    let changelog = include_str!("../../../../CHANGELOG.md");
    let manifest = include_str!("../../../../Cargo.toml");
    let version = manifest
        .split("[workspace.package]")
        .nth(1)
        .and_then(|section| section.split("version = \"").nth(1))
        .and_then(|rest| rest.split('"').next());
    assert!(
        version.is_some(),
        "the workspace version should be readable"
    );
    let Some(version) = version else {
        return;
    };

    let heading = format!("## {version} ");
    assert!(
        changelog.contains(&heading),
        "the workspace version {version} needs a changelog section, or a release \
         of it would publish empty notes"
    );

    // Every other section must be a version that was actually tagged. The
    // newest one is the release being prepared; the rest are history.
    let sections: Vec<&str> = changelog
        .lines()
        .filter(|line| line.starts_with("## "))
        .collect();
    assert!(
        sections
            .first()
            .is_some_and(|first| first.starts_with(&heading)),
        "the newest section should be the version about to ship, got {:?}",
        sections.first()
    );
    // The rest are history, and history only goes one way. Two sections at
    // the same version, or an older one above a newer one, means a local bump
    // grew its own section instead of folding into the release being
    // prepared -- which is how 1.0.7 and 1.0.8 came to advertise changes
    // nobody could download.
    let mut seen: Vec<[u64; 3]> = Vec::new();
    for line in &sections {
        let Some(number) = line.split_whitespace().nth(1) else {
            panic!("changelog section without a version: {line:?}");
        };
        let parts: Vec<u64> = number
            .split('.')
            .filter_map(|part| part.parse::<u64>().ok())
            .collect();
        assert_eq!(
            parts.len(),
            3,
            "changelog sections are headed by a three-part version, got {number:?}"
        );
        let parsed = [parts[0], parts[1], parts[2]];
        if let Some(previous) = seen.last() {
            assert!(
                parsed < *previous,
                "changelog sections run newest first with no repeats; \
                 {number} follows {previous:?}"
            );
        }
        seen.push(parsed);
    }
}

#[test]
fn the_usage_guide_documents_the_shortcuts_the_build_implements() {
    // Every "how do I measure thickness" question used to have nowhere to
    // point: the manifest homepage 404s and the README linked nothing. A guide
    // that invents bindings would be worse than none, so this checks that the
    // ones it lists are the ones the code reads.
    let usage = include_str!("../../../../docs/USAGE.md");
    let readme = include_str!("../../../../README.md");
    assert!(
        readme.contains("docs/USAGE.md"),
        "the guide has to be reachable from the README"
    );

    let editor = repo_source_file("src/app/app_mesh_editor.rs");
    let sculpt = repo_source_file("src/app/app_sculpt.rs");
    let dialogs = repo_source_file("src/app/app_dialogs.rs");

    assert!(
        usage.contains("**Ctrl+A**") && editor.contains("egui::Key::A"),
        "select-all is documented and implemented"
    );
    assert!(
        usage.contains("**Delete** or **Backspace**")
            && editor.contains("egui::Key::Delete")
            && editor.contains("egui::Key::Backspace"),
        "the delete bindings are documented and implemented"
    );
    assert!(
        usage.contains("**Ctrl+Z**") && editor.contains("egui::Key::Z"),
        "undo is documented and implemented"
    );
    assert!(
        usage.contains("**Ctrl+O**") && dialogs.contains("egui::Key::O"),
        "open is documented and implemented"
    );
    assert!(
        usage.contains("Add / Remove brush | **1**") && sculpt.contains("egui::Key::Num1"),
        "the brush selector is documented and implemented"
    );
    assert!(
        usage.contains("occluview-cli close-holes"),
        "the CLI subcommands should be listed where a user can find them"
    );
}

/// Every key the viewer consumes, written the way the guide writes it.
///
/// The guide is checked in both directions against this table: a key the build
/// reads and the guide never names leaves an operator guessing, and a key the
/// guide names that nothing reads is an invention.
const VIEWER_KEY_BINDINGS: &[(&str, &str)] = &[
    ("A", "**Ctrl+A**"),
    ("Backspace", "**Backspace**"),
    ("Delete", "**Delete**"),
    ("Enter", "**Enter**"),
    ("Escape", "**Esc**"),
    ("F", "**F**"),
    ("Num1", "**1**"),
    ("Num2", "**2**"),
    ("O", "**Ctrl+O**"),
    ("Y", "**Ctrl+Y**"),
    ("Z", "**Ctrl+Z**"),
];

/// The `egui::Key::NAME` variants this crate reads outside its test modules.
fn keys_the_viewer_binds() -> std::collections::BTreeSet<String> {
    let mut sources = Vec::new();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    collect_rust_source_files(&root, &mut sources)
        .unwrap_or_else(|error| panic!("cannot walk the viewer sources: {error}"));

    let mut keys = std::collections::BTreeSet::new();
    for path in sources {
        // Test modules name keys they never bind, which is the point of them.
        if path
            .components()
            .any(|part| part.as_os_str().to_string_lossy().contains("tests"))
        {
            continue;
        }
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
        for (offset, _) in text.match_indices("egui::Key::") {
            let name: String = text[offset + "egui::Key::".len()..]
                .chars()
                .take_while(|character| character.is_alphanumeric() || *character == '_')
                .collect();
            if !name.is_empty() {
                keys.insert(name);
            }
        }
    }
    keys
}

#[test]
fn the_usage_guide_names_every_key_the_viewer_binds_and_no_others() {
    // The guide used to promise a key the build does not have (`F` framing a
    // measurement) and skip one it does (Shift+Middle-click). Both are the same
    // failure: the guide and the code drifted with nothing comparing them.
    //
    // What this cannot check is meaning. `F` really is bound, and the guide
    // really did say it framed the cut when it flips which half is kept; no
    // assertion over text catches that. It catches the keys, so the prose is
    // the only part a reviewer has to re-read.
    let usage = include_str!("../../../../docs/USAGE.md");
    let bound = keys_the_viewer_binds();

    for name in &bound {
        let documented = VIEWER_KEY_BINDINGS
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, spelling)| *spelling);
        let Some(spelling) = documented else {
            panic!(
                "the viewer binds egui::Key::{name} and the guide has no entry for it;                  add it to docs/USAGE.md and to VIEWER_KEY_BINDINGS"
            );
        };
        assert!(
            usage.contains(spelling),
            "the viewer binds egui::Key::{name}, so docs/USAGE.md should say {spelling}"
        );
    }

    for (name, spelling) in VIEWER_KEY_BINDINGS {
        assert!(
            bound.contains(*name),
            "docs/USAGE.md documents {spelling} but nothing in the viewer reads              egui::Key::{name} any more"
        );
    }
}

#[test]
fn the_guide_mentions_f_only_where_something_binds_f() {
    // `F` is bound exactly once in the viewer -- flipping the planted cut --
    // and once in the Explorer preview window, where it frames the model. The
    // guide claimed it in a third place, under Measuring, where no key F
    // exists. Sections are the finest grain a text guard can work at, so pin
    // the sections.
    let usage = include_str!("../../../../docs/USAGE.md");
    let sections_naming_f: Vec<&str> = usage
        .split("\n## ")
        .skip(1)
        .filter(|section| section.contains("**F**"))
        .filter_map(|section| section.lines().next())
        .collect();
    assert_eq!(
        sections_naming_f,
        vec!["The cut view", "Windows Explorer"],
        "F belongs to the planted cut and to the Explorer preview; anywhere else          it is a shortcut the build does not have"
    );

    let cut = repo_source_file("src/app/app_cut_measure.rs");
    assert!(
        cut.contains("self.cut_view.is_planted()") && cut.contains("egui::Key::F"),
        "the cut view is where F is read, and only while the disc is planted"
    );
    let preview = repo_source_file("../occluview-shell/src/com/preview/window.rs");
    assert!(
        preview.contains("const VK_F: u32 = 0x46;"),
        "the Explorer preview is the other place the guide may name F"
    );
}
