//! Structural guards over the workspace source tree itself.

use super::*;
use std::path::{Path, PathBuf};

/// True for files rustc reaches only through an explicit `mod` declaration.
///
/// Crate roots are named by `Cargo.toml`, and cargo auto-discovers every
/// `src/bin/*.rs` as its own binary target, so neither needs one.
fn needs_a_module_declaration(path: &Path) -> bool {
    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    if file_name == "lib.rs" || file_name == "main.rs" {
        return false;
    }
    let in_bin_directory = path.parent().is_some_and(|parent| {
        parent.file_name().is_some_and(|name| name == "bin")
            && parent.parent().is_some_and(|source_root| {
                source_root.file_name().is_some_and(|name| name == "src")
            })
    });
    !in_bin_directory
}

/// The module name `path` would be declared under, and the directory the
/// declaring file lives in.
///
/// `foo/mod.rs` is module `foo` declared beside the `foo` directory;
/// `foo/bar.rs` is module `bar` declared inside `foo`.
fn declaration_site(path: &Path) -> Option<(String, PathBuf)> {
    let file_name = path.file_name().and_then(|name| name.to_str())?;
    let directory = path.parent()?;
    if file_name == "mod.rs" {
        let module_name = directory.file_name().and_then(|name| name.to_str())?;
        Some((module_name.to_owned(), directory.parent()?.to_path_buf()))
    } else {
        let module_name = path.file_stem().and_then(|stem| stem.to_str())?;
        Some((module_name.to_owned(), directory.to_path_buf()))
    }
}

/// The files rustc consults for `mod <name>;`, in both module layouts: the
/// directory's own `mod.rs` or crate root, and the 2018-style `<dir>.rs`
/// sitting next to the directory.
fn module_files_for(directory: &Path) -> Vec<PathBuf> {
    let mut candidates = vec![
        directory.join("mod.rs"),
        directory.join("lib.rs"),
        directory.join("main.rs"),
    ];
    if let (Some(parent), Some(directory_name)) = (directory.parent(), directory.file_name()) {
        if let Some(directory_name) = directory_name.to_str() {
            candidates.push(parent.join(format!("{directory_name}.rs")));
        }
    }
    candidates
}

fn declares_module(candidate: &Path, module_name: &str) -> bool {
    let Ok(source) = std::fs::read_to_string(candidate) else {
        return false;
    };
    source.contains(&format!("mod {module_name};"))
        || source.contains(&format!("mod {module_name} {{"))
}

/// `#[path = "sibling.rs"]` lets any file in the same directory adopt another,
/// which is how the oversized modules in this crate park their tests.
fn adopted_by_a_sibling(path: &Path, source_files: &[PathBuf]) -> bool {
    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let needle = format!("#[path = \"{file_name}\"]");
    source_files
        .iter()
        .filter(|sibling| sibling.as_path() != path && sibling.parent() == path.parent())
        .any(|sibling| {
            std::fs::read_to_string(sibling).is_ok_and(|source| source.contains(&needle))
        })
}

fn is_reachable(path: &Path, source_files: &[PathBuf]) -> bool {
    let Some((module_name, directory)) = declaration_site(path) else {
        return false;
    };
    let declared = module_files_for(&directory)
        .into_iter()
        .filter(|candidate| candidate.as_path() != path)
        .any(|candidate| declares_module(&candidate, &module_name));
    declared || adopted_by_a_sibling(path, source_files)
}

/// The `src` directory of every crate in the workspace.
///
/// Only those files reach rustc through a `mod` declaration. Cargo discovers
/// `build.rs`, `tests/`, `examples/` and `benches/` roots on its own.
fn workspace_source_roots(crates_directory: &Path) -> Result<Vec<PathBuf>, String> {
    let entries = std::fs::read_dir(crates_directory)
        .map_err(|error| format!("cannot read {}: {error}", crates_directory.display()))?;
    let mut roots = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| format!("cannot read entry: {error}"))?;
        let source_root = entry.path().join("src");
        if source_root.is_dir() {
            roots.push(source_root);
        }
    }
    roots.sort();
    Ok(roots)
}

#[test]
fn every_source_file_is_named_by_a_module_declaration() {
    // A `.rs` file that no `mod` declaration names is never handed to rustc.
    // It compiles nowhere, so it cannot fail to compile, and it keeps looking
    // like live code: `gltf/tests.rs` sat unread for its whole life and let a
    // stack-overflow bug through, and `edit_mode/selection_cache.rs` had drifted
    // so far from the API it called that wiring it in broke the build.
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
    let collected = (|| -> Result<(), String> {
        for source_root in workspace_source_roots(&workspace_root.join("crates"))? {
            collect_rust_source_files(&source_root, &mut source_files)?;
        }
        Ok(())
    })();
    assert!(collected.is_ok(), "source audit failed: {collected:?}");
    assert!(
        source_files.len() > 100,
        "walked only {} files; the audit is not seeing the workspace",
        source_files.len()
    );

    let orphans: Vec<String> = source_files
        .iter()
        .filter(|path| needs_a_module_declaration(path))
        .filter(|path| !is_reachable(path, &source_files))
        .map(|path| path.display().to_string())
        .collect();

    assert!(
        orphans.is_empty(),
        "these files are never compiled; add `mod <name>;` to the parent module \
         or delete them:\n{}",
        orphans.join("\n")
    );
}

#[test]
fn the_orphan_guard_recognises_both_module_layouts() {
    let root = std::env::temp_dir().join(format!("occluview-module-graph-{}", std::process::id()));
    let outcome = (|| -> Result<(), String> {
        let write = |path: &Path, body: &str| -> Result<(), String> {
            std::fs::write(path, body).map_err(|error| format!("cannot write fixture: {error}"))
        };
        std::fs::create_dir_all(root.join("with_mod_rs"))
            .map_err(|error| format!("cannot create fixture: {error}"))?;
        std::fs::create_dir_all(root.join("sibling_style"))
            .map_err(|error| format!("cannot create fixture: {error}"))?;
        write(
            &root.join("lib.rs"),
            "mod with_mod_rs;\nmod sibling_style;\n",
        )?;
        write(&root.join("with_mod_rs/mod.rs"), "mod named;\n")?;
        write(&root.join("with_mod_rs/named.rs"), "fn named() {}\n")?;
        write(&root.join("with_mod_rs/unnamed.rs"), "fn unnamed() {}\n")?;
        write(&root.join("sibling_style.rs"), "mod adopted_child;\n")?;
        write(
            &root.join("sibling_style/adopted_child.rs"),
            "fn child() {}\n",
        )?;
        write(
            &root.join("sibling_style/host.rs"),
            "#[path = \"guest.rs\"]\nmod guest;\n",
        )?;
        write(&root.join("sibling_style/guest.rs"), "fn guest() {}\n")?;
        Ok(())
    })();
    assert!(outcome.is_ok(), "fixture setup failed: {outcome:?}");

    let mut source_files = Vec::new();
    let collected = collect_rust_source_files(&root, &mut source_files);
    assert!(collected.is_ok(), "fixture walk failed: {collected:?}");

    let mut orphans: Vec<String> = source_files
        .iter()
        .filter(|path| needs_a_module_declaration(path))
        .filter(|path| !is_reachable(path, &source_files))
        .filter_map(|path| path.file_name().and_then(|name| name.to_str()))
        .map(str::to_owned)
        .collect();
    orphans.sort();
    let _ = std::fs::remove_dir_all(&root);

    // `host.rs` is itself undeclared in this fixture, which is the point: the
    // guard reports it while still crediting the `#[path]` child it adopts.
    assert_eq!(orphans, vec!["host.rs", "unnamed.rs"]);
}

/// Home-directory names that appear in this workspace on purpose.
///
/// Every one of them is a fixture standing in for "somebody's home", written
/// so a reader can tell at a glance that no real machine is involved. A name
/// outside this list in an absolute path is a path that only resolves on the
/// machine it was written on.
const FIXTURE_HOME_NAMES: &[&str] = &["clinic", "me", "operator", "user"];

/// The absolute-path prefixes a home directory can follow, per platform.
const HOME_PREFIXES: &[&str] = &["/home/", "/Users/", "C:\\Users\\", "C:\\\\Users\\\\"];

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

#[test]
fn no_source_file_carries_a_path_from_one_machine() {
    // Seven diagnostic dumps once wrote their PNGs into an absolute path from
    // one tool session, so the tests were unrunnable for everyone including
    // their author. A test fixture later carried a real home directory into a
    // public repository the same way. Both are the same mistake: an absolute
    // path that resolves on exactly one machine.
    //
    // This file is the one place the offending shapes may be written out, so
    // the guard skips itself instead of disguising its own literals.
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
    assert!(collected.is_ok(), "source audit failed: {collected:?}");

    let mut offenders = Vec::new();
    for path in source_files {
        if path.ends_with("primary_ui_tests/source_tree.rs") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for prefix in HOME_PREFIXES {
            for (offset, _) in text.match_indices(prefix) {
                let Some(segment) = segment_after(&text, offset, prefix) else {
                    continue;
                };
                if !FIXTURE_HOME_NAMES.contains(&segment.as_str()) {
                    offenders.push(format!("{}: {prefix}{segment}", path.display()));
                }
            }
        }
        for (offset, _) in text.match_indices("/tmp/") {
            let Some(segment) = segment_after(&text, offset, "/tmp/") else {
                continue;
            };
            if looks_like_a_session_scratchpad(&segment) {
                offenders.push(format!("{}: /tmp/{segment}", path.display()));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "source files must not hard-code a path that resolves on one machine; \
         use a fixture home from FIXTURE_HOME_NAMES, CARGO_TARGET_TMPDIR or \
         std::env::temp_dir():\n{}",
        offenders.join("\n")
    );
}

#[test]
fn the_architecture_note_counts_the_unsafe_in_occluview_formats() {
    // The note said "single memmap2 unsafe site" and stayed that way when a
    // second, unrelated Win32 call arrived. An architecture document is read
    // precisely for claims like this one, so it has to be countable.
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir.parent().and_then(Path::parent);
    let Some(workspace_root) = workspace_root else {
        panic!("app crate should live under the workspace crates directory");
    };

    let mut sources = Vec::new();
    let formats = workspace_root.join("crates/occluview-formats/src");
    collect_rust_source_files(&formats, &mut sources)
        .unwrap_or_else(|error| panic!("cannot walk occluview-formats: {error}"));

    let mut sites = Vec::new();
    for path in sources {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for (number, line) in text.lines().enumerate() {
            if line.contains("unsafe {") {
                sites.push(format!("{}:{}", path.display(), number + 1));
            }
        }
    }

    assert_eq!(
        sites.len(),
        2,
        "occluview-formats should hold exactly the two unsafe blocks the \
         architecture note describes -- the mmap and the drive-type query:\n{}",
        sites.join("\n")
    );

    let architecture = std::fs::read_to_string(workspace_root.join("docs/ARCHITECTURE.md"))
        .unwrap_or_else(|error| panic!("cannot read the architecture note: {error}"));
    assert!(
        architecture.contains("Two `unsafe` sites"),
        "the architecture note should say how many unsafe sites the crate has"
    );
}
