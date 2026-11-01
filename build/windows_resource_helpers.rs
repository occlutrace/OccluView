// Helpers shared by the three Windows build scripts that embed VERSIONINFO.
//
// `occluview-app`, `occluview-cli` and `occluview-shell` each describe a different
// binary — a GUI executable, a console executable and a DLL say different things
// on their Properties pages — but locating the SDK's `rc.exe`, walking the SDK
// directories and parsing the crate version into a `FILEVERSION` tuple are the
// same work. Three copies of the version tuple are exactly what ships a
// Properties page that disagrees with the installer, so the helpers live here
// and each build script `include!`s this file into a private module.
//
// The header is plain comments rather than `//!` because an included file cannot
// open with an inner doc comment.
//
// An including build script must sit two directories below the repository root.

use std::env;
use std::fs;
use std::path::PathBuf;

pub(crate) fn find_resource_compiler() -> Result<PathBuf, Box<dyn std::error::Error>> {
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

pub(crate) fn find_in_path(command: &str) -> Option<PathBuf> {
    let paths = env::var_os("PATH")?;
    env::split_paths(&paths)
        .map(|path| path.join(command))
        .find(|path| path.is_file())
}

pub(crate) fn windows_kits_roots() -> Vec<PathBuf> {
    ["ProgramFiles(x86)", "ProgramFiles"]
        .into_iter()
        .filter_map(env::var_os)
        .map(PathBuf::from)
        .collect()
}

pub(crate) fn version_tuple(version: &str) -> (u16, u16, u16) {
    let mut parts = version.split('.');
    let major = parse_version_part(parts.next());
    let minor = parse_version_part(parts.next());
    let patch = parse_version_part(parts.next());
    (major, minor, patch)
}

pub(crate) fn parse_version_part(part: Option<&str>) -> u16 {
    part.and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(0)
}
