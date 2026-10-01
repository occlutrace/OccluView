//! Repository-level contract primitives.
//!
//! The three functions here answer one question each about a file every release
//! has to keep in step: the version in the workspace manifest, the version
//! `Cargo.lock` records per package, and the `ProductVersion` fallback the `WiX`
//! installer defines. They parse text, so the crate has no dependencies and the
//! repository-contract tests can assert the three agree without building any
//! OccluView crate.
#![forbid(unsafe_code)]

/// The `version` of `[workspace.package]` in a workspace `Cargo.toml`.
#[must_use]
pub fn workspace_package_version(cargo_toml: &str) -> Option<&str> {
    let section = cargo_toml.split("[workspace.package]").nth(1)?;
    toml_quoted_value(section, "version")
}

/// The `version` `Cargo.lock` records for `package_name`.
#[must_use]
pub fn cargo_lock_package_version<'a>(cargo_lock: &'a str, package_name: &str) -> Option<&'a str> {
    let package_line = format!("name = \"{package_name}\"");
    cargo_lock
        .split("[[package]]")
        .find(|block| block.lines().any(|line| line.trim() == package_line))
        .and_then(|block| toml_quoted_value(block, "version"))
}

/// The `ProductVersion` fallback a `WiX` installer defines, which is what the
/// package uses when the build does not pass a version in.
#[must_use]
pub fn wix_product_version(wxs: &str) -> Option<&str> {
    let marker = "<?define ProductVersion = \"";
    let rest = wxs.get(wxs.find(marker)? + marker.len()..)?;
    rest.get(..rest.find('"')?)
}

fn toml_quoted_value<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            continue;
        }
        let Some(rest) = trimmed.strip_prefix(key) else {
            continue;
        };
        let rest = rest.trim_start();
        let Some(rest) = rest.strip_prefix('=') else {
            continue;
        };
        let rest = rest.trim_start();
        let Some(rest) = rest.strip_prefix('"') else {
            continue;
        };
        return rest.get(..rest.find('"')?);
    }
    None
}
