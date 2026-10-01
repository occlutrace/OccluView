//! Windows resource embedding for the `occluview.exe` GUI binary.
//!
//! The resource definitions stay local to this script; the SDK plumbing it shares
//! with `crates/occluview-cli/build.rs` (the console binary) and
//! `crates/occluview-shell/build.rs` (the Explorer DLL) lives in
//! `build/windows_resource_helpers.rs` and is `include!`d below.

#![allow(clippy::print_stdout)]

use fluent_syntax::ast::Entry;
use std::collections::BTreeSet;
use std::env;
use std::error::Error;
use std::fs;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};
use std::process::Command;

/// VERSIONINFO helpers shared with the other two Windows build scripts.
mod versioninfo {
    include!("../../build/windows_resource_helpers.rs");
}

/// Fluent catalogs owned by `occluview-i18n`, relative to this crate's root.
/// The app reads them here to build its key-parity gate and message-id macro.
const CATALOGS: &str = "../occluview-i18n/i18n";

fn main() -> Result<(), Box<dyn Error>> {
    // A change to the shared helpers must rebuild this script.
    println!("cargo:rerun-if-changed=../../build/windows_resource_helpers.rs");

    println!("cargo:rerun-if-changed=assets/windows/occluview.ico");
    println!("cargo:rerun-if-changed={CATALOGS}");

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    // i18n contract gate: every `occluview-i18n/i18n/*.ftl` catalog must carry
    // exactly the `en` key set. Test-time validation covers variables, variant
    // names and plurals; this fails the BUILD on drift so a broken or
    // half-added catalog never ships in any binary.
    check_i18n_key_parity(&manifest_dir)?;
    generate_message_id_macro(&manifest_dir)?;

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

    let rc_exe = versioninfo::find_resource_compiler()?;
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

fn generate_message_id_macro(manifest_dir: &Path) -> Result<(), Box<dyn Error>> {
    let catalog_path = manifest_dir.join(CATALOGS).join("en.ftl");
    let source = fs::read_to_string(&catalog_path)?;
    let resource = fluent_syntax::parser::parse(source.as_str()).map_err(|(_, errors)| {
        let details = errors
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("; ");
        io::Error::new(
            ErrorKind::InvalidData,
            format!("English Fluent catalog does not parse: {details}"),
        )
    })?;
    let ids = message_ids(resource.body);
    let output_dir = PathBuf::from(env::var("OUT_DIR")?);
    fs::write(output_dir.join("message_ids.rs"), render_macro(&ids))?;
    println!("cargo:rerun-if-changed={}", catalog_path.display());
    Ok(())
}

fn message_ids(entries: Vec<Entry<&str>>) -> BTreeSet<String> {
    let mut ids = BTreeSet::new();
    for entry in entries {
        if let Entry::Message(message) = entry {
            if message.value.is_some() {
                ids.insert(message.id.name.to_owned());
            }
            for attribute in message.attributes {
                ids.insert(format!("{}.{}", message.id.name, attribute.id.name));
            }
        }
    }
    ids
}

fn render_macro(ids: &BTreeSet<String>) -> String {
    let mut arms = ids
        .iter()
        .map(|id| format!("    ({id:?}) => {{ $crate::i18n::MessageId::new({id:?}) }};"))
        .collect::<Vec<_>>();
    arms.push(
        "    ($unknown:literal) => { compile_error!(concat!(\"unknown English Fluent message id: \", $unknown)); };"
            .to_owned(),
    );
    format!(
        "macro_rules! message_id {{\n{}\n}}\npub(crate) use message_id;\n",
        arms.join("\n")
    )
}

/// Fail the build when any `occluview-i18n/i18n/*.ftl` catalog drifts from the
/// `en` key set (missing/extra keys). Only top-level `key =` lines count:
/// comments, indented continuations and select syntax never start at
/// column zero with a key-shaped head.
fn check_i18n_key_parity(manifest_dir: &Path) -> Result<(), Box<dyn Error>> {
    let dir = manifest_dir.join(CATALOGS);
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
        .ok_or("occluview-i18n/i18n/en.ftl is missing")?;
    if baseline.1.is_empty() {
        return Err("occluview-i18n/i18n/en.ftl carries no keys".into());
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
            "occluview-i18n/i18n catalog key drift vs en (see also the test-time contract):\n{}",
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

fn windows_resource_script(icon_path: &Path) -> Result<String, Box<dyn Error>> {
    let version = env::var("CARGO_PKG_VERSION")?;
    let version_parts = versioninfo::version_tuple(&version);
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
