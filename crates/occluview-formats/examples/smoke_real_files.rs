//! Smoke-test the loaders against REAL dental files on disk.
//!
//! Run with:
//!   cargo run -p occluview-formats --example smoke_real_files -- <file> [<file> ...]
//!
//! Each file is parsed end-to-end through dispatch_by_extension and we print
//! the mesh stats (vertex/triangle counts, has-colors, bbox dims). This is the
//! honest reality check: synthetic tests do not catch real-scanner quirks.

// This is a CLI tool: stdout/stderr is the entire point, and usize->f64 cast
// for a size-in-MB display is intentional.
#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::cast_precision_loss,
    clippy::doc_markdown
)]

use occluview_formats::dispatch_by_extension;
use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("usage: smoke_real_files <file> [<file> ...]");
        return ExitCode::from(2);
    }

    let mut failures = 0usize;
    for path_str in &args {
        let path = Path::new(path_str);
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .unwrap_or_default();

        let label = path_str;
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                println!("[FAIL] {label}\n         read error: {e}");
                failures += 1;
                continue;
            }
        };

        let size_mb = bytes.len() as f64 / 1_048_576.0;
        match dispatch_by_extension(&ext, &bytes) {
            Ok(mut mesh) => {
                let bbox = mesh.bbox();
                let [w, h, d] = bbox.dimensions_mm();
                let tris = mesh.triangle_count();
                let verts = mesh.vertices().len();
                let kind = if mesh.is_point_cloud() {
                    "points"
                } else {
                    "mesh"
                };
                println!(
                    "[ OK ] {label}\n         {size_mb:.1} MB  ext={ext}  {kind}  tris={tris}  verts={verts}  colors={}  bbox={:.1} x {:.1} x {:.1} mm",
                    if mesh.has_vertex_colors() { "yes" } else { "no" },
                    w.as_mm(),
                    h.as_mm(),
                    d.as_mm(),
                );
            }
            Err(e) => {
                println!("[FAIL] {label}\n         {size_mb:.1} MB  ext={ext}  error: {e}");
                failures += 1;
            }
        }
    }

    if failures == 0 {
        println!("\nall files parsed successfully");
        ExitCode::SUCCESS
    } else {
        println!("\n{failures} file(s) failed to parse");
        ExitCode::FAILURE
    }
}
