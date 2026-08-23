//! Windows VERSIONINFO for the two console binaries, `occluview-cli.exe` and
//! `occluview-hps-export.exe`.
//!
//! Both ship in support bundles, so their Properties pages must name the
//! product and version like the GUI binary does. The resource plumbing is
//! kept in sync with `crates/occluview-app/build.rs` by hand: build scripts
//! cannot share code without a dedicated build-dependency crate, and this
//! much duplication is the cheaper contract.

#![allow(clippy::print_stdout)]

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let target_is_windows = env::var_os("CARGO_CFG_WINDOWS").is_some();
    if !target_is_windows {
        return Ok(());
    }

    let out_dir = PathBuf::from(env::var("OUT_DIR")?);
    let rc_exe = find_resource_compiler()?;

    for binary in [
        BinaryResource {
            bin_name: "occluview-cli",
            original_filename: "occluview-cli.exe",
            description: "OccluView headless CLI",
        },
        BinaryResource {
            bin_name: "occluview-hps-export",
            original_filename: "occluview-hps-export.exe",
            description: "OccluView HPS export tool",
        },
    ] {
        let rc_path = out_dir.join(format!("{}.rc", binary.bin_name));
        let res_path = out_dir.join(format!("{}.res", binary.bin_name));
        fs::write(&rc_path, exe_resource_script(&binary)?)?;

        let status = Command::new(&rc_exe)
            .arg("/nologo")
            .arg(format!("/fo{}", res_path.display()))
            .arg(&rc_path)
            .status()?;
        if !status.success() {
            return Err(format!("rc.exe failed while compiling {}", rc_path.display()).into());
        }

        println!(
            "cargo:rustc-link-arg-bin={}={}",
            binary.bin_name,
            res_path.display()
        );
    }
    Ok(())
}

struct BinaryResource {
    bin_name: &'static str,
    original_filename: &'static str,
    description: &'static str,
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

fn exe_resource_script(binary: &BinaryResource) -> Result<String, Box<dyn std::error::Error>> {
    let version = env::var("CARGO_PKG_VERSION")?;
    let version_parts = version_tuple(&version);
    Ok(format!(
        r#"1 VERSIONINFO
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
      VALUE "FileDescription", "{description}\0"
      VALUE "FileVersion", "{version}\0"
      VALUE "InternalName", "{internal_name}\0"
      VALUE "LegalCopyright", "Copyright (c) Dental Cloud Technologies and contributors\0"
      VALUE "OriginalFilename", "{original_filename}\0"
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
        description = binary.description,
        internal_name = binary.bin_name,
        original_filename = binary.original_filename,
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
