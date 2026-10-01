//! Asserts that the dependency graph respects `[workspace.metadata.layer-ranks]`.

#![allow(clippy::expect_used)] // a missing workspace field is a test failure

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;

#[test]
fn workspace_dependencies_match_the_declared_layer_map() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = manifest_dir
        .parent()
        .and_then(Path::parent)
        .expect("crate manifest is inside the workspace");
    let output = Command::new("cargo")
        .current_dir(root)
        .args(["metadata", "--locked", "--format-version", "1", "--no-deps"])
        .output()
        .expect("cargo metadata runs");
    assert!(
        output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let metadata: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("cargo metadata returns valid JSON");
    let packages = metadata["packages"]
        .as_array()
        .expect("workspace packages are present");
    let package_names: BTreeSet<&str> = packages
        .iter()
        .filter_map(|package| package["name"].as_str())
        .collect();
    let layers = metadata["metadata"]["layer-ranks"]
        .as_object()
        .expect("workspace layer ranks are present");
    let declared_layers: BTreeMap<&str, u64> = layers
        .iter()
        .map(|(package, rank)| {
            (
                package.as_str(),
                rank.as_u64()
                    .expect("each package has a numeric layer rank"),
            )
        })
        .collect();

    assert_eq!(
        declared_layers.keys().copied().collect::<BTreeSet<_>>(),
        package_names,
        "every workspace crate has one declared layer rank"
    );

    for package in packages {
        let name = package["name"].as_str().expect("package name is present");
        let package_rank = declared_layers
            .get(name)
            .copied()
            .expect("every workspace crate has a layer rank");
        let dependencies = package["dependencies"]
            .as_array()
            .expect("package dependencies are present")
            .iter()
            .filter_map(|dependency| dependency["name"].as_str())
            .filter(|dependency| package_names.contains(dependency));
        for dependency in dependencies {
            let dependency_rank = declared_layers
                .get(dependency)
                .copied()
                .expect("every workspace dependency has a layer rank");
            assert!(
                dependency_rank < package_rank,
                "{name} (layer {package_rank}) may not depend on {dependency} (layer {dependency_rank})"
            );
        }
    }
}
