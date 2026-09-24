use super::*;
use std::path::PathBuf;

#[test]
fn source_collector_ignores_generated_target_directories() {
    let root = std::env::temp_dir().join(format!("occluview-source-scan-{}", std::process::id()));
    let collected = (|| -> Result<Vec<PathBuf>, String> {
        std::fs::create_dir_all(root.join("target"))
            .map_err(|error| format!("cannot create fixture: {error}"))?;
        std::fs::write(root.join("kept.rs"), "fn kept() {}\n")
            .map_err(|error| format!("cannot write kept fixture: {error}"))?;
        std::fs::write(root.join("target/generated.rs"), "fn generated() {}\n")
            .map_err(|error| format!("cannot write generated fixture: {error}"))?;
        let mut files = Vec::new();
        collect_rust_source_files(&root, &mut files)?;
        Ok(files)
    })();
    let _ = std::fs::remove_dir_all(&root);

    assert!(collected.is_ok(), "source collection failed: {collected:?}");
    let Ok(files) = collected else {
        return;
    };
    assert!(files.iter().any(|path| path.ends_with("kept.rs")));
    assert!(!files.iter().any(|path| path.ends_with("generated.rs")));
}
