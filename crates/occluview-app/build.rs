//! Windows resource embedding for the `occluview.exe` GUI binary.
//!
//! Keep the corresponding resource definitions synchronized with:
//! `crates/occluview-cli/build.rs` (the console binaries) and
//! `crates/occluview-shell/build.rs` (the Explorer DLL).

#![allow(clippy::print_stdout)]

use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=assets/windows/occluview.ico");
    println!("cargo:rerun-if-changed=i18n");

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    // i18n contract gate: every `i18n/*.ftl` catalog must carry exactly
    // the `en` key set. Test-time validation covers variables, variant
    // names and plurals; this fails the BUILD on drift so a broken or
    // half-added catalog never ships in any binary.
    check_i18n_key_parity(&manifest_dir)?;

    let target_is_windows = env::var_os("CARGO_CFG_WINDOWS").is_some();
    if !target_is_windows {
        return Ok(());
    }

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    let out_dir = PathBuf::from(env::var("OUT_DIR")?);
    let icon_path = manifest_dir.join("assets/windows/occluview.ico");
    let rc_path = out_dir.join("occluview.rc");
    let res_path = out_dir.join("occluview.res");

    fs::write(&rc_path, windows_resource_script(&icon_path)?)?;

    let rc_exe = find_resource_compiler()?;
    let status = Command::new(rc_exe)
        .arg("/nologo")
        .arg(format!("/fo{}", res_path.display()))
        .arg(&rc_path)
        .status()?;
    if !status.success() {
        return Err(format!("rc.exe failed while compiling {}", rc_path.display()).into());
    }

    println!("cargo:rustc-link-arg-bin=occluview={}", res_path.display());
    Ok(())
}

/// Fail the build when any `i18n/*.ftl` catalog drifts from the `en` key
/// set (missing/extra keys). Only top-level `key =` lines count:
/// comments, indented continuations and select syntax never start at
/// column zero with a key-shaped head.
fn check_i18n_key_parity(manifest_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let dir = manifest_dir.join("i18n");
    let mut catalogs: Vec<(String, BTreeSet<String>)> = Vec::new();
    let mut entries = fs::read_dir(&dir)
        .map_err(|error| format!("cannot read {}: {error}", dir.display()))?
        .collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("ftl") {
            continue;
        }
        let tag = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .ok_or_else(|| format!("unreadable catalog name: {}", path.display()))?
            .to_owned();
        let source = fs::read_to_string(&path)?;
        catalogs.push((tag, ftl_top_level_keys(&source)));
    }
    let baseline = catalogs
        .iter()
        .find(|(tag, _)| tag == "en")
        .ok_or("i18n/en.ftl is missing")?;
    if baseline.1.is_empty() {
        return Err("i18n/en.ftl carries no keys".into());
    }
    let mut problems = Vec::new();
    for (tag, keys) in &catalogs {
        if tag == "en" {
            continue;
        }
        for key in baseline.1.difference(keys) {
            problems.push(format!("{tag}: missing key '{key}'"));
        }
        for key in keys.difference(&baseline.1) {
            problems.push(format!("{tag}: unused key '{key}'"));
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        problems.sort();
        Err(format!(
            "i18n catalog key drift vs en (see also the test-time contract):\n{}",
            problems.join("\n")
        )
        .into())
    }
}

fn ftl_top_level_keys(source: &str) -> BTreeSet<String> {
    source
        .lines()
        .filter_map(|line| {
            let head = line.split('=').next()?.trim_end();
            if head.is_empty()
                || head.starts_with('#')
                || head.starts_with(char::is_whitespace)
                || !head
                    .chars()
                    .all(|cell| cell.is_ascii_alphanumeric() || cell == '-' || cell == '_')
            {
                return None;
            }
            Some(head.to_owned())
        })
        .collect()
}

fn find_resource_compiler() -> Result<PathBuf, Box<dyn std::error::Error>> {
    if let Some(rc) = env::var_os("RC") {
        return Ok(PathBuf::from(rc));
    }

    for candidate in ["rc.exe", "llvm-rc.exe", "llvm-rc"] {
        if let Some(path) = find_in_path(candidate) {
            return Ok(path);
        }
    }

    for base in windows_kits_roots() {
        let bin_root = base.join("Windows Kits").join("10").join("bin");
        let Ok(entries) = fs::read_dir(bin_root) else {
            continue;
        };
        let mut candidates = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path().join("x64").join("rc.exe"))
            .filter(|path| path.exists())
            .collect::<Vec<_>>();
        candidates.sort();
        if let Some(path) = candidates.pop() {
            return Ok(path);
        }
    }

    Err("Windows SDK resource compiler rc.exe was not found".into())
}

fn find_in_path(command: &str) -> Option<PathBuf> {
    let paths = env::var_os("PATH")?;
    env::split_paths(&paths)
        .map(|path| path.join(command))
        .find(|path| path.is_file())
}

fn windows_kits_roots() -> Vec<PathBuf> {
    ["ProgramFiles(x86)", "ProgramFiles"]
        .into_iter()
        .filter_map(env::var_os)
        .map(PathBuf::from)
        .collect()
}

fn windows_resource_script(icon_path: &Path) -> Result<String, Box<dyn std::error::Error>> {
    let version = env::var("CARGO_PKG_VERSION")?;
    let version_parts = version_tuple(&version);
    let icon = icon_path.display().to_string().replace('\\', "\\\\");
    Ok(format!(
        r#"1 ICON "{icon}"

1 VERSIONINFO
 FILEVERSION {major},{minor},{patch},0
 PRODUCTVERSION {major},{minor},{patch},0
 FILEFLAGSMASK 0x3fL
 FILEFLAGS 0x0L
 FILEOS 0x40004L
 FILETYPE 0x1L
 FILESUBTYPE 0x0L
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "CompanyName", "Dental Cloud Technologies\0"
      VALUE "FileDescription", "OccluView 3D Viewer\0"
      VALUE "FileVersion", "{version}\0"
      VALUE "InternalName", "occluview\0"
      VALUE "LegalCopyright", "Copyright (c) Dental Cloud Technologies and contributors\0"
      VALUE "OriginalFilename", "occluview.exe\0"
      VALUE "ProductName", "OccluView 3D Viewer\0"
      VALUE "ProductVersion", "{version}\0"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#,
        major = version_parts.0,
        minor = version_parts.1,
        patch = version_parts.2,
    ))
}

fn version_tuple(version: &str) -> (u16, u16, u16) {
    let mut parts = version.split('.');
    let major = parse_version_part(parts.next());
    let minor = parse_version_part(parts.next());
    let patch = parse_version_part(parts.next());
    (major, minor, patch)
}

fn parse_version_part(part: Option<&str>) -> u16 {
    part.and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(0)
}
