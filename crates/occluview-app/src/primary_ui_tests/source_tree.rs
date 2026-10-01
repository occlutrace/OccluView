//! Guards over the workspace source tree itself.
//!
//! A path that resolves on one machine, a tool session's scratch directory, or
//! a scan named after its case makes a checkout unrunnable for everyone else
//! and can publish patient data. These are repository contracts that no
//! runtime test can see, so they are checked by reading the tracked sources.

use super::*;
use std::path::Path;

/// Home-directory names that appear in this workspace on purpose.
///
/// Every one of them is a fixture standing in for "somebody's home", written
/// so a reader can tell at a glance that no real machine is involved. A name
/// outside this list in an absolute path is a path that only resolves on the
/// machine it was written on.
const FIXTURE_HOME_NAMES: &[&str] = &["clinic", "me", "operator", "user"];

/// The absolute-path prefixes a home directory can follow, per platform.
const HOME_PREFIXES: &[&str] = &["/home/", "/Users/", "C:\\Users\\", "C:\\\\Users\\\\"];

/// The shortest digit run that reads as a case number rather than a version,
/// a colour or an item count.
const MIN_CASE_NUMBER_DIGITS: usize = 5;

/// The first path segment after `prefix` at `offset`, if there is one.
fn segment_after(text: &str, offset: usize, prefix: &str) -> Option<String> {
    let rest = text.get(offset + prefix.len()..)?;
    let segment: String = rest
        .chars()
        .take_while(|character| {
            character.is_alphanumeric()
                || *character == '_'
                || *character == '-'
                || *character == '.'
        })
        .collect();
    (!segment.is_empty()).then_some(segment)
}

/// True for a `/tmp` segment that looks like one tool run's scratch directory
/// rather than a fixture: those carry both a dash and a digit
/// (`/tmp/<tool>-1101`), while `/tmp/a.stl` and `/tmp/xdg-state` do not.
fn looks_like_a_session_scratchpad(segment: &str) -> bool {
    segment.contains('-') && segment.chars().any(|character| character.is_ascii_digit())
}

/// The `#<digits>` case numbers in `text`.
///
/// The private scan corpus names each file after its case
/// (`upper_jaw_pretreatment_#148438659.stl`), so a source file carrying one has
/// copied a patient identifier into the repository.
fn case_numbers_in(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    for (offset, _) in text.match_indices('#') {
        let digits: String = text
            .get(offset + 1..)
            .unwrap_or_default()
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        if digits.len() >= MIN_CASE_NUMBER_DIGITS {
            found.push(format!("#{digits}"));
        }
    }
    found
}

/// Every private shape `text` contains, rendered as `path: token`.
///
/// The workspace guard calls this per tracked source file; the planted-string
/// test calls it directly, so the rules are covered without writing a real
/// private path into the repository.
fn privacy_offenders(path: &Path, text: &str) -> Vec<String> {
    let mut offenders = Vec::new();
    for &prefix in HOME_PREFIXES {
        for (offset, _) in text.match_indices(prefix) {
            let Some(segment) = segment_after(text, offset, prefix) else {
                continue;
            };
            if !FIXTURE_HOME_NAMES.contains(&segment.as_str()) {
                offenders.push(format!("{}: {prefix}{segment}", path.display()));
            }
        }
    }
    for (offset, _) in text.match_indices("/tmp/") {
        let Some(segment) = segment_after(text, offset, "/tmp/") else {
            continue;
        };
        if looks_like_a_session_scratchpad(&segment) {
            offenders.push(format!("{}: /tmp/{segment}", path.display()));
        }
    }
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    for case_number in case_numbers_in(text).into_iter().chain(case_numbers_in(file_name)) {
        offenders.push(format!("{}: {case_number}", path.display()));
    }
    offenders
}

/// The guard's own file is the one place the forbidden shapes may be written
/// out, so the workspace walk skips it.
fn is_guard_source(path: &Path) -> bool {
    path.ends_with("primary_ui_tests/source_tree.rs")
}

#[test]
fn no_source_file_carries_a_private_path_or_a_patient_identifier() {
    // A fixture once carried a real home directory into a public repository,
    // and diagnostic dumps wrote their PNGs into one tool session's scratch
    // directory, which made those tests unrunnable for everyone including
    // their author. Same mistake twice: a path that resolves on exactly one
    // machine. The corpus filenames add a third: a case number is patient
    // data even when it only appears in a string.
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir.parent().and_then(Path::parent);
    assert!(
        workspace_root.is_some(),
        "app crate should live under the workspace crates directory"
    );
    let Some(workspace_root) = workspace_root else {
        return;
    };
    let mut source_files = Vec::new();
    let collected = collect_rust_source_files(&workspace_root.join("crates"), &mut source_files);
    assert!(collected.is_ok(), "source scan failed: {collected:?}");
    assert!(
        source_files.len() > 100,
        "walked only {} files; the scan is not seeing the workspace",
        source_files.len()
    );

    let mut offenders = Vec::new();
    for path in source_files {
        if is_guard_source(&path) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        offenders.extend(privacy_offenders(&path, &text));
    }

    assert!(
        offenders.is_empty(),
        "tracked sources must not carry an absolute machine path, a tool \
         session scratch path, or a patient case number:\n{}",
        offenders.join("\n")
    );
}

#[test]
fn the_privacy_guard_flags_planted_paths_and_case_numbers() {
    let root = std::env::temp_dir().join(format!("occluview-privacy-{}", std::process::id()));
    let outcome = (|| -> Result<(), String> {
        std::fs::create_dir_all(&root)
            .map_err(|error| format!("cannot create fixture directory: {error}"))?;
        let write = |name: &str, body: &str| -> Result<(), String> {
            std::fs::write(root.join(name), body)
                .map_err(|error| format!("cannot write fixture: {error}"))
        };
        write(
            "machine.rs",
            "const ROOT: &str = \"/home/zer0ltrnce/occluview\";\n",
        )?;
        write(
            "scratch.rs",
            "const OUT: &str = \"/tmp/nerfio-1101/overlay.png\";\n",
        )?;
        write(
            "patient.rs",
            "const SCAN: &str = \"upper_jaw_pretreatment_#148438659.stl\";\n",
        )?;
        write(
            "clean.rs",
            "const TMP: &str = \"/tmp/a.stl\";\nconst HOME: &str = \"/home/user\";\n",
        )?;
        Ok(())
    })();
    assert!(outcome.is_ok(), "fixture setup failed: {outcome:?}");

    let mut files = Vec::new();
    let collected = collect_rust_source_files(&root, &mut files);
    assert!(collected.is_ok(), "fixture walk failed: {collected:?}");

    let mut flagged = Vec::new();
    for path in &files {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        flagged.extend(privacy_offenders(path, &text));
    }
    let _ = std::fs::remove_dir_all(&root);

    let count_for = |name: &str| flagged.iter().filter(|line| line.contains(name)).count();
    assert_eq!(
        count_for("machine.rs"),
        1,
        "a planted absolute machine path must be flagged: {flagged:?}"
    );
    assert_eq!(
        count_for("scratch.rs"),
        1,
        "a planted tool-session scratch path must be flagged: {flagged:?}"
    );
    assert_eq!(
        count_for("patient.rs"),
        1,
        "a planted case number must be flagged: {flagged:?}"
    );
    assert_eq!(
        count_for("clean.rs"),
        0,
        "fixture-shaped paths must not be flagged: {flagged:?}"
    );
}
