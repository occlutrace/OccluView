//! Checks the release notes and operator documentation shipped with the app.

use super::*;
use std::path::PathBuf;

#[test]
fn the_changelog_starts_with_the_current_version_and_descends_without_repeats() {
    let changelog = include_str!("../../../../CHANGELOG.md");
    let current = env!("CARGO_PKG_VERSION");
    let expected_heading = format!("## {current} ");
    let sections: Vec<_> = changelog
        .lines()
        .filter(|line| line.starts_with("## "))
        .collect();

    let version_sections: Vec<_> = sections
        .iter()
        .copied()
        .filter(|heading| !heading.starts_with("## Unreleased"))
        .collect();
    assert!(
        version_sections
            .first()
            .is_some_and(|heading| heading.starts_with(&expected_heading)),
        "the first changelog section must describe the current package version {current}"
    );

    let mut previous = None;
    for heading in version_sections {
        let Some(version) = heading.split_whitespace().nth(1).and_then(parse_version) else {
            panic!("changelog section has no three-part version: {heading:?}");
        };
        if let Some(previous) = previous {
            assert!(
                version < previous,
                "changelog sections must descend without repeats: {heading:?}"
            );
        }
        previous = Some(version);
    }
}

/// A three-part version, with or without a leading `v`.
fn parse_version(raw: &str) -> Option<[u64; 3]> {
    let parts: Vec<u64> = raw
        .trim_start_matches('v')
        .split('.')
        .map(str::parse::<u64>)
        .collect::<Result<_, _>>()?;
    (parts.len() == 3).then(|| [parts[0], parts[1], parts[2]])
}

/// Every key the viewer consumes, written the way the README writes it.
///
/// The README key list is product text. Checking both sources catches a
/// documented shortcut with no handler and a handler an operator cannot find.
const VIEWER_KEY_BINDINGS: &[(&str, &[&str])] = &[
    ("A", &["**A**", "**Ctrl+A**"]),
    ("Backspace", &["**Backspace**"]),
    ("C", &["**C**"]),
    ("Delete", &["**Delete**"]),
    ("E", &["**E**"]),
    ("Enter", &["**Enter**"]),
    ("Escape", &["**Esc**"]),
    ("F", &["**F**"]),
    ("F1", &["**F1**"]),
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
    let readme = include_str!("../../../../README.md");
    let bound = keys_the_viewer_binds();

    for name in &bound {
        let spelling = VIEWER_KEY_BINDINGS
            .iter()
            .find(|(key, _)| key == name)
            .and_then(|(_, spellings)| spellings.first().copied());
        let Some(spelling) = spelling else {
            panic!("README has no key-table entry for the bound egui::Key::{name}");
        };
        assert!(
            readme.contains(spelling),
            "README must document egui::Key::{name} as {spelling}"
        );
    }

    for (name, spellings) in VIEWER_KEY_BINDINGS {
        assert!(
            bound.contains(*name),
            "README documents {spellings:?}, but the viewer does not bind egui::Key::{name}"
        );
    }

    for token in readme.split("**").skip(1).step_by(2) {
        if !looks_like_a_key(token) {
            continue;
        }
        let bold = format!("**{token}**");
        let known = VIEWER_KEY_BINDINGS
            .iter()
            .any(|(_, spellings)| spellings.contains(&bold.as_str()))
            || NON_KEYBOARD_BINDINGS.contains(&token);
        assert!(known, "README has an unregistered key binding {bold}");
    }
}

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

fn looks_like_a_key(token: &str) -> bool {
    !token.is_empty()
        && !token.contains(' ')
        && token.split('+').all(|part| {
            part.chars()
                .next()
                .is_some_and(|first| first.is_ascii_uppercase() || first.is_ascii_digit())
        })
}
