//! Windows VERSIONINFO for the console binary, `occluview-cli.exe`.
//!
//! The binary ships in support bundles, so its Properties page must name the
//! product and version like the GUI binary does. Only the resource script below is
//! local; the SDK plumbing it shares with the other two build scripts — locating
//! `rc.exe` and parsing the crate version — comes from
//! `build/windows_resource_helpers.rs`.

#![allow(clippy::print_stdout)]

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

/// VERSIONINFO helpers shared with the other two Windows build scripts.
mod versioninfo {
    include!("../../build/windows_resource_helpers.rs");
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // A change to the shared helpers must rebuild this script.
    println!("cargo:rerun-if-changed=../../build/windows_resource_helpers.rs");

    let target_is_windows = env::var_os("CARGO_CFG_WINDOWS").is_some();
    if !target_is_windows {
        return Ok(());
    }

    let out_dir = PathBuf::from(env::var("OUT_DIR")?);
    let rc_exe = versioninfo::find_resource_compiler()?;

    let binary = BinaryResource {
        bin_name: "occluview-cli",
        original_filename: "occluview-cli.exe",
        description: "OccluView headless CLI",
    };
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
    Ok(())
}

struct BinaryResource {
    bin_name: &'static str,
    original_filename: &'static str,
    description: &'static str,
}

fn exe_resource_script(binary: &BinaryResource) -> Result<String, Box<dyn std::error::Error>> {
    let version = env::var("CARGO_PKG_VERSION")?;
    let version_parts = versioninfo::version_tuple(&version);
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
