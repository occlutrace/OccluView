//! `occluview-cli` - headless CLI.
//!
//! Subcommands:
//!   - `thumbnail <file> [-o out.png] [--size N]` - render a thumbnail via
//!     the same file-backed offscreen path the Explorer shell extension uses,
//!     so a correct PNG here means a correct thumbnail in Explorer.
//!   - `convert <file> -o out.{stl|ply|obj}` - transcode a mesh into a common
//!     exchange format. Keeps geometry, vertex colors (PLY/OBJ), normals, and
//!     UVs where the destination format supports them.
//!   - `info <file> [file...]` - print format, vertex/triangle counts, bbox,
//!     colors, UVs, texture. Multiple files print per-file stats + an
//!     aggregate scene bbox (upper+lower arch case).
//!   - `help` - show usage.

// CLI tool: stdout/stderr is the entire point.
#![allow(clippy::print_stdout, clippy::print_stderr)]

mod export;

use anyhow::{anyhow, Context, Result};
use occluview_formats::dispatch::{read_file_with_key_provider, read_files_with_key_provider};
use occluview_formats::hps::RuntimeHpsKeyProvider;
use std::path::{Path, PathBuf};

fn main() {
    install_tracing();
    let exit_code = match run() {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("{error:#}");
            1
        }
    };
    std::process::exit(exit_code);
}

/// Send the workspace's `tracing` output to stderr.
///
/// This binary is the freedesktop thumbnailer the Debian package installs, so
/// when a clinic's file manager shows placeholder cubes instead of scans, this
/// is where the reason comes out. The crate declared `tracing-subscriber` and
/// never installed one, which meant every `tracing::warn!` from the format,
/// thumbnail and render crates went nowhere and `RUST_LOG` did nothing --
/// while the subscriber's whole dependency tree still shipped in the binary
/// and in its attribution.
///
/// Default is `warn`, so ordinary runs stay quiet; `RUST_LOG=debug` turns the
/// diagnosis on. stderr, because that is already this tool's progress channel
/// and the thumbnailer contract only cares about the output file.
fn install_tracing() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_writer(std::io::stderr)
        .compact()
        .try_init();
}

fn run() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let subcommand = args.next().unwrap_or_else(|| {
        print_usage();
        "help".to_string()
    });

    match subcommand.as_str() {
        "thumbnail" => cmd_thumbnail(&mut args),
        "convert" => cmd_convert(&mut args),
        "close-holes" => cmd_close_holes(&mut args),
        "info" => cmd_info(&mut args),
        "help" | "--help" | "-h" => {
            print_usage();
            Ok(())
        }
        "--version" | "-V" => {
            println!("occluview-cli {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        other => {
            print_usage();
            Err(anyhow!("unknown subcommand: {other}"))
        }
    }
}

/// `thumbnail <file> [-o out.png] [--size N]`
fn cmd_thumbnail(args: &mut impl Iterator<Item = String>) -> Result<()> {
    let file: PathBuf = args
        .next()
        .ok_or_else(|| anyhow!("thumbnail: missing <file> argument"))?
        .into();
    let mut output: Option<PathBuf> = None;
    let mut size: u16 = 256;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-o" | "--output" => {
                output = Some(PathBuf::from(
                    args.next().ok_or_else(|| anyhow!("-o requires a path"))?,
                ));
            }
            "--size" => {
                size = args
                    .next()
                    .ok_or_else(|| anyhow!("--size requires a number"))?
                    .parse()
                    .context("--size must be a number")?;
            }
            other => return Err(anyhow!("unknown flag: {other}")),
        }
    }

    let out_path = output.unwrap_or_else(|| {
        let mut p = file.clone();
        p.set_extension("png");
        p
    });

    eprintln!("Rendering {size}x{size} thumbnail...");
    // Use the same infallible, placeholder-backed path Explorer's shell
    // extension uses: a corrupt / unsupported / encrypted-without-key / over-
    // budget file yields a neutral placeholder cube (with a "!" badge for
    // broken files) instead of an error. This matters for the freedesktop
    // thumbnailer contract (`Exec=occluview-cli thumbnail %i -o %o`): a non-zero
    // exit or missing output makes the file manager show a broken-image glyph,
    // which is exactly the "broken thumbnails" we want to avoid. Any fallback
    // reason is tracing-logged inside the provider; `RUST_LOG=debug` prints it
    // (see `install_tracing`).
    let pixels = occluview_thumbnail::render_thumbnail_file_or_placeholder(
        &file,
        occluview_render::ThumbnailSpec {
            size_px: size,
            ..Default::default()
        },
    );

    eprintln!("Writing {}...", out_path.display());
    let img = image::RgbaImage::from_raw(u32::from(size), u32::from(size), pixels)
        .ok_or_else(|| anyhow!("failed to create image buffer"))?;
    img.save(&out_path)
        .with_context(|| format!("writing {}", out_path.display()))?;

    eprintln!("Done: {}", out_path.display());
    std::process::exit(0);
}

/// `convert <file> -o output.{stl|ply|obj}`
fn cmd_convert(args: &mut impl Iterator<Item = String>) -> Result<()> {
    let input: PathBuf = args
        .next()
        .ok_or_else(|| anyhow!("convert: missing <file> argument"))?
        .into();
    let mut output: Option<PathBuf> = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-o" | "--output" => {
                output = Some(PathBuf::from(
                    args.next().ok_or_else(|| anyhow!("-o requires a path"))?,
                ));
            }
            other => return Err(anyhow!("unknown flag: {other}")),
        }
    }

    let output = output.ok_or_else(|| anyhow!("convert: missing -o <output-path>"))?;
    let format = export::convert_file(&input, &output)?;
    eprintln!(
        "Converted {} -> {} ({format:?})",
        input.display(),
        output.display()
    );
    Ok(())
}

/// `close-holes <file> -o out.stl [--limit-mm N]` - run the whole-mesh Close
/// Holes pipeline headlessly (STL loads as soup) and write the closed result.
/// Prints the honest edit report so a soup input can be verified end to end.
fn cmd_close_holes(args: &mut impl Iterator<Item = String>) -> Result<()> {
    let input: PathBuf = args
        .next()
        .ok_or_else(|| anyhow!("close-holes: missing <file> argument"))?
        .into();
    let mut output: Option<PathBuf> = None;
    let mut limit_mm: Option<f32> = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-o" | "--output" => {
                output = Some(PathBuf::from(
                    args.next().ok_or_else(|| anyhow!("-o requires a path"))?,
                ));
            }
            "--limit-mm" => {
                limit_mm = Some(
                    args.next()
                        .ok_or_else(|| anyhow!("--limit-mm requires a number"))?
                        .parse()
                        .context("--limit-mm must be a number")?,
                );
            }
            other => return Err(anyhow!("unknown flag: {other}")),
        }
    }
    let output = output.ok_or_else(|| anyhow!("close-holes: missing -o <output-path>"))?;

    let report = export::close_holes_file(&input, &output, limit_mm)?;
    println!("File:              {}", input.display());
    println!(
        "Input:             verts={} tris={}",
        report.input_vertices, report.input_triangles
    );
    println!(
        "Output:            verts={} tris={}",
        report.output_vertices, report.output_triangles
    );
    println!("Closed holes:      {}", report.filled_holes);
    println!("Healed nicks:      {}", report.healed_rims);
    println!("Skipped (border):  {}", report.skipped_border_rims);
    println!("Skipped (oversize):{}", report.skipped_oversize_rims);
    println!("Skipped (damaged): {}", report.skipped_damaged_rims);
    println!("Wrote:             {}", output.display());
    Ok(())
}

/// `info <file> [file...]` - print mesh statistics. When multiple files are
/// given, prints per-file stats plus an aggregate scene bbox.
fn cmd_info(args: &mut impl Iterator<Item = String>) -> Result<()> {
    let files: Vec<PathBuf> = args.map(PathBuf::from).collect();
    if files.is_empty() {
        return Err(anyhow!("info: missing <file> argument"));
    }

    // Single-file fast path keeps the existing output format unchanged.
    if files.len() == 1 {
        return cmd_info_one(&files[0]);
    }

    // Multi-file: per-file summary + scene aggregate.
    let scene = read_files_with_key_provider(&files, &RuntimeHpsKeyProvider)
        .map_err(|(path, e)| anyhow!("{}: {}", path.display(), e))?;

    for (i, entry) in scene.meshes().iter().enumerate() {
        let m = &entry.mesh;
        let bbox = m.bbox_uncached();
        let [w, h, d] = bbox.dimensions_mm();
        println!(
            "[{}/{}] {}  verts={} tris={} kind={} bbox={:.1}x{:.1}x{:.1}mm",
            i + 1,
            scene.meshes().len(),
            files
                .get(i)
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
            m.vertices().len(),
            m.triangle_count(),
            if m.is_point_cloud() { "cloud" } else { "mesh" },
            w.as_mm(),
            h.as_mm(),
            d.as_mm(),
        );
    }

    let scene_bbox = scene.bbox();
    if !scene_bbox.is_empty() {
        let [w, h, d] = scene_bbox.dimensions_mm();
        println!(
            "Scene bbox: {:.2} x {:.2} x {:.2} mm  ({})",
            w.as_mm(),
            h.as_mm(),
            d.as_mm(),
            files.len()
        );
    }
    Ok(())
}

/// Single-file info (the original output format).
fn cmd_info_one(file: &Path) -> Result<()> {
    let mut mesh = read_file_with_key_provider(file, &RuntimeHpsKeyProvider)
        .with_context(|| format!("loading {}", file.display()))?;

    let bbox = mesh.bbox();
    let [w, h, d] = bbox.dimensions_mm();

    println!("File:       {}", file.display());
    println!(
        "Format:     {}",
        file.extension().and_then(|e| e.to_str()).unwrap_or("?")
    );
    println!(
        "Kind:       {}",
        if mesh.is_point_cloud() {
            "point cloud"
        } else {
            "triangle mesh"
        }
    );
    println!("Vertices:   {}", mesh.vertices().len());
    println!("Triangles:  {}", mesh.triangle_count());
    println!(
        "Colors:     {}",
        if mesh.has_vertex_colors() {
            "yes"
        } else {
            "no"
        }
    );
    println!("UVs:        {}", if mesh.has_uvs() { "yes" } else { "no" });
    println!(
        "Texture:    {}",
        if mesh.texture().is_some() {
            "yes"
        } else {
            "no"
        }
    );
    println!(
        "Bbox:       {:.2} x {:.2} x {:.2} mm",
        w.as_mm(),
        h.as_mm(),
        d.as_mm()
    );
    println!(
        "Bbox range: [{:.2}, {:.2}, {:.2}] .. [{:.2}, {:.2}, {:.2}]",
        bbox.min.x, bbox.min.y, bbox.min.z, bbox.max.x, bbox.max.y, bbox.max.z,
    );
    Ok(())
}

fn print_usage() {
    eprintln!(
        "occluview-cli - headless OccluView\n\
         \n\
         USAGE:\n    \
         occluview-cli <SUBCOMMAND> [ARGS]\n\
         \n\
         SUBCOMMANDS:\n    \
         thumbnail <file> [-o out.png] [--size N]   Render a thumbnail (same path as the Explorer extension)\n    \
         convert   <file> -o output.{{stl|ply|obj}}   Convert a mesh into STL / PLY / OBJ\n    \
         close-holes <file> -o out.stl [--limit-mm N] Close holes (whole-mesh) and write the result\n    \
         info      <file>                          Print format / counts / bbox\n    \
         help                                       Show this message\n    \
         --version | -V                             Print the version and exit"
    );
}

#[cfg(test)]
mod tests {
    #[test]
    fn version_flag_is_recognised_and_advertised() {
        let source = include_str!("main.rs");
        assert!(
            source.contains("\"--version\" | \"-V\""),
            "--version must dispatch instead of falling into the unknown-subcommand error"
        );
        assert!(
            source.contains("--version | -V"),
            "the usage text should advertise the flag"
        );
    }

    #[test]
    fn thumbnail_cli_uses_file_backed_render_path() {
        let source = include_str!("main.rs");
        let start = source.find("fn cmd_thumbnail(");
        assert!(start.is_some(), "missing cmd_thumbnail");
        let Some(start) = start else {
            return;
        };
        let end = source[start..].find("/// `info <file>");
        assert!(
            end.is_some(),
            "missing info command after thumbnail command"
        );
        let Some(end) = end else {
            return;
        };
        let thumbnail = &source[start..start + end];

        assert!(
            thumbnail
                .contains("occluview_thumbnail::render_thumbnail_file_or_placeholder("),
            "CLI thumbnails should use the file-backed, placeholder-backed path shared with Explorer \
             so corrupt/unsupported files still produce a PNG (freedesktop thumbnailer contract)"
        );
        assert!(
            !thumbnail.contains("std::fs::read(&file)"),
            "CLI thumbnails should not read large files into memory before rendering"
        );
        assert!(
            !thumbnail.contains("read_file_with_key_provider(&file"),
            "CLI thumbnails should not parse once for console stats and again for rendering"
        );
    }

    #[test]
    fn convert_cli_routes_through_export_module() {
        let source = include_str!("main.rs");
        assert!(source.contains("\"convert\" => cmd_convert(&mut args)"));
        assert!(source.contains("export::convert_file(&input, &output)?;"));
        assert!(source.contains("output.{stl|ply|obj}"));
    }
}
