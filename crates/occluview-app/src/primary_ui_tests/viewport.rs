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

#[test]
fn live_window_uses_matching_msaa_for_custom_wgpu_viewport() {
    let source = app_bootstrap_source();
    let live_viewport = include_str!("../live_viewport.rs");
    let native_options = function_source(
        source,
        "fn native_options(preflight: &GraphicsPreflight) -> eframe::NativeOptions {",
    );

    assert!(
        native_options.contains("multisampling: preflight.live_sample_count"),
        "eframe MSAA must use the startup-selected sample count"
    );
    assert!(
        live_viewport.contains("Renderer::with_shared_device_sample_count(")
            && live_viewport.contains("sample_count: u16"),
        "custom live viewport pipelines must receive the eframe render-pass sample count"
    );
    assert!(
        source.contains("let live_sample_count = graphics_preflight.live_sample_count")
            && source.contains("from_render_state(state, live_sample_count)"),
        "the selected count must cross startup into the custom viewport"
    );
}
