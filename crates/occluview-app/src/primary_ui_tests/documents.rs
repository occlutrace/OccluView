//! Guards over the documents the build ships with.
//!
//! The changelog against the version being prepared, and the README against
//! the keys the viewer actually binds. Both drift silently: nothing fails to
//! compile when an operator instruction describes a shortcut the build lacks.

use super::*;

#[test]
fn the_readme_is_the_only_repository_guide_for_operators() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir.parent().and_then(Path::parent);
    assert!(
        workspace_root.is_some(),
        "app crate should live under the workspace crates directory"
    );
    let Some(workspace_root) = workspace_root else {
        return;
    };

    assert!(workspace_root.join("README.md").is_file());
    assert!(
        !workspace_root.join("docs").exists(),
        "operator instructions belong in README.md, not a repository docs tree"
    );
}

#[test]
fn the_changelog_only_names_versions_that_can_be_released() {
    // Release notes come from the section matching the current version.
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
    // The rest are history, and history goes one way. A repeat, or an older
    // section above a newer one, means a local bump grew its own section
    // instead of folding into the release being prepared.
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

    // Ordering is not yet the rule the test name promises. A section below the
    // newest claims something was released, so a tag has to exist for it. Tags
    // come from git; a source tarball has none, and there the ordering above is
    // all there is.
    let Some(tags) = repository_tags() else {
        return;
    };
    // Only from the first tagged version onward: sections older than the day
    // tagging started describe releases this repository has no record of.
    let Some(first_tagged) = tags.iter().filter_map(|tag| parse_version(tag)).min() else {
        return;
    };
    for line in sections.iter().skip(1) {
        let Some(number) = line.split_whitespace().nth(1) else {
            continue;
        };
        let Some(parsed) = parse_version(number) else {
            continue;
        };
        if parsed < first_tagged {
            continue;
        }
        assert!(
            tags.iter().any(|tag| tag == &format!("v{number}")),
            "the changelog has a section for {number}, which was never tagged; \
             an untagged section publishes nothing and advertises a version \
             nobody can download"
        );
    }
}

/// A three-part version, with or without a leading `v`.
fn parse_version(raw: &str) -> Option<[u64; 3]> {
    let parts: Vec<u64> = raw
        .trim_start_matches('v')
        .split('.')
        .map(|part| part.parse::<u64>().ok())
        .collect::<Option<Vec<u64>>>()?;
    (parts.len() == 3).then(|| [parts[0], parts[1], parts[2]])
}

/// The tags of this repository, or `None` outside a git checkout.
fn repository_tags() -> Option<Vec<String>> {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir.parent().and_then(Path::parent)?;
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(workspace_root)
        .arg("tag")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let tags: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|tag| !tag.is_empty())
        .map(str::to_owned)
        .collect();
    (!tags.is_empty()).then_some(tags)
}

#[test]
fn the_readme_documents_the_shortcuts_the_build_implements() {
    // A documented shortcut that the build does not bind is worse than no
    // shortcut, so cross-check the public operator surface against the code.
    let readme = include_str!("../../../../README.md");

    let editor = repo_source_file("src/app/app_mesh_editor.rs");
    let sculpt = repo_source_file("src/app/app_sculpt.rs");
    let dialogs = repo_source_file("src/app/app_dialogs.rs");

    assert!(
        readme.contains("**Ctrl+A**") && editor.contains("egui::Key::A"),
        "select-all is documented and implemented"
    );
    assert!(
        readme.contains("**Delete** or **Backspace**")
            && editor.contains("egui::Key::Delete")
            && editor.contains("egui::Key::Backspace"),
        "the delete bindings are documented and implemented"
    );
    assert!(
        readme.contains("**Ctrl+Z**") && editor.contains("egui::Key::Z"),
        "undo is documented and implemented"
    );
    assert!(
        readme.contains("**Ctrl+O**") && dialogs.contains("egui::Key::O"),
        "open is documented and implemented"
    );
    assert!(
        readme.contains("**1** chooses Add/Remove") && sculpt.contains("egui::Key::Num1"),
        "the brush selector is documented and implemented"
    );
    assert!(
        readme.contains("occluview-cli close-holes"),
        "the CLI subcommands should be listed where a user can find them"
    );
}

#[test]
fn the_controls_catalogue_names_the_wired_gestures() {
    let catalogue = repo_source_file("src/interaction_hints.rs");

    for gesture in [
        "Ctrl+O",
        "RMB drag",
        "MMB drag",
        "Ctrl+A",
        "Delete",
        "Ctrl+Shift+Z",
        "Shift+wheel",
        "Ctrl+wheel",
        "F",
        "Esc",
    ] {
        assert!(
            catalogue.contains(gesture),
            "the controls catalogue should name {gesture}"
        );
    }

    for section in [
        "Navigation",
        "Mesh Editing",
        "Sculpt",
        "Layers and Explorer Preview",
    ] {
        assert!(
            catalogue.contains(section),
            "the controls catalogue should include the {section} section"
        );
    }
}

/// Every key the viewer consumes, written the way the README writes it.
///
/// The README is checked in both directions against this table: a key the
/// build reads and README never names leaves an operator guessing, and a key
/// it names that nothing reads is an invention.
const VIEWER_KEY_BINDINGS: &[(&str, &[&str])] = &[
    ("A", &["**A**", "**Ctrl+A**"]),
    ("Backspace", &["**Backspace**"]),
    ("C", &["**C**"]),
    ("Delete", &["**Delete**"]),
    ("E", &["**E**"]),
    ("Enter", &["**Enter**"]),
    ("Escape", &["**Esc**"]),
    ("F", &["**F**"]),
    ("M", &["**M**"]),
    ("Num1", &["**1**"]),
    ("Num2", &["**2**"]),
    ("O", &["**Ctrl+O**"]),
    ("T", &["**T**"]),
    ("Y", &["**Ctrl+Y**"]),
    ("Z", &["**Ctrl+Z**", "**Ctrl+Shift+Z**"]),
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
fn the_readme_names_every_key_the_viewer_binds_and_no_others() {
    // README and code drift both ways with nothing comparing them: `F` framing
    // a measurement, which the build does not do, and Shift+Middle-click,
    // which it does and the guide skipped.
    //
    // Meaning is out of reach here. `F` really is bound, and the guide really
    // did say it framed the cut when it flips which half is kept. Keys are
    // covered, so the prose is the only part a reviewer has to re-read.
    let readme = include_str!("../../../../README.md");
    let bound = keys_the_viewer_binds();

    for name in &bound {
        let documented = VIEWER_KEY_BINDINGS
            .iter()
            .find(|(key, _)| key == name)
            .and_then(|(_, spellings)| spellings.first().copied());
        let Some(spelling) = documented else {
            panic!(
                "the viewer binds egui::Key::{name} and README has no entry for it; add it to README.md and VIEWER_KEY_BINDINGS"
            );
        };
        assert!(
            readme.contains(spelling),
            "the viewer binds egui::Key::{name}, so README.md should say {spelling}"
        );
    }

    for (name, spellings) in VIEWER_KEY_BINDINGS {
        assert!(
            bound.contains(*name),
            "README.md documents {spellings:?} but nothing in the viewer \
             reads egui::Key::{name} any more"
        );
    }

    // The other direction has to read the README, not the table: checking only
    // the spellings already listed here says nothing about a shortcut somebody
    // invented in the prose. Every bold token in the README that looks like a
    // key has to be one of them.
    for token in readme.split("**").skip(1).step_by(2) {
        if !looks_like_a_key(token) {
            continue;
        }
        let bold = format!("**{token}**");
        let known = VIEWER_KEY_BINDINGS
            .iter()
            .any(|(_, spellings)| spellings.contains(&bold.as_str()))
            || NON_KEYBOARD_BINDINGS.contains(&token);
        assert!(
            known,
            "README.md documents {bold}, which nothing in the viewer binds; \
             add the binding or drop the line"
        );
    }
}

/// Bold tokens that are real bindings the keyboard table does not cover.
///
/// `W` is read by the Explorer preview window rather than the viewer. The
/// pointer chords are verified where they are implemented, in the layer
/// interaction guard, and `Shift` on its own is a modifier held during a drag,
/// not a shortcut.
const NON_KEYBOARD_BINDINGS: &[&str] = &[
    "Help",
    "W",
    "Shift",
    "Shift+wheel",
    "Ctrl/Command+drag",
    "RMB click",
    "Ctrl+wheel",
    "Ctrl+Middle-click",
    "Ctrl+Shift+Middle-click",
    "Shift+Middle-click",
];

/// Whether a bold token in the README is naming a key rather than emphasising a
/// word.
///
/// Keys are written as a modifier chain of capitalised words or a single
/// character: `Ctrl+A`, `Esc`, `F`, `1`. Anything containing a space, or
/// starting lowercase, is prose.
fn looks_like_a_key(token: &str) -> bool {
    !token.is_empty()
        && !token.contains(' ')
        && token.split('+').all(|part| {
            part.chars()
                .next()
                .is_some_and(|first| first.is_ascii_uppercase() || first.is_ascii_digit())
        })
}

#[test]
fn the_readme_mentions_f_only_where_something_binds_f() {
    // `F` is bound exactly once in the viewer -- flipping the planted cut --
    // and once in the Explorer preview window, where it frames the model. The
    // README must not claim it in a third place, under Measuring, where no key F
    // exists. Sections are the finest grain a text guard can work at, so pin
    // the sections.
    let readme = include_str!("../../../../README.md");
    let sections_naming_f: Vec<&str> = readme
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

#[test]
fn the_readme_points_operators_to_the_complete_controls_reference() {
    let readme = include_str!("../../../../README.md");

    for phrase in [
        "**Help**",
        "complete keyboard and mouse reference",
        "**Shift+wheel** changes Sculpt brush size",
        "**Ctrl+wheel** changes Sculpt brush intensity",
        "**Shift** erases an Align exclusion region",
        "**Ctrl/Command+drag** rotates a scan in Align",
        "**F** flips the kept half",
        "**W** toggles wireframe",
    ] {
        assert!(
            readme.contains(phrase),
            "README should explicitly document {phrase}"
        );
    }
}
