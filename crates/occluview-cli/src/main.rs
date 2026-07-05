//! `occluview-cli` - headless CLI.
//!
//! Subcommands:
//!   - `thumbnail <file> [-o out.png] [--size N]` - render a thumbnail via
//!     the same offscreen path the Explorer shell extension uses. The exact
//!     same code path (`render_thumb::render_thumbnail`), so a correct PNG here
//!     means a correct thumbnail in Explorer.
//!   - `info <file>` - print format, vertex/triangle counts, bbox, colors.
//!   - `help` - show usage.

// CLI tool: stdout/stderr is the entire point.
#![allow(clippy::print_stdout, clippy::print_stderr)]

use anyhow::{anyhow, Context, Result};
use occluview_formats::dispatch::read_file;
use std::path::PathBuf;

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let subcommand = args.next().unwrap_or_else(|| {
        print_usage();
        "help".to_string()
    });

    match subcommand.as_str() {
        "thumbnail" => cmd_thumbnail(&mut args),
        "info" => cmd_info(&mut args),
        "help" | "--help" | "-h" => {
            print_usage();
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

    eprintln!("Loading {}...", file.display());
    let mesh = read_file(&file).with_context(|| format!("loading {}", file.display()))?;
    eprintln!(
        "  {} vertices, {} triangles, {}",
        mesh.vertices().len(),
        mesh.triangle_count(),
        if mesh.is_point_cloud() {
            "point cloud"
        } else {
            "triangle mesh"
        }
    );

    eprintln!("Rendering {size}x{size} thumbnail...");
    let pixels = occluview_shell::render_thumbnail(
        file.extension()
            .and_then(|e| e.to_str())
            .ok_or_else(|| anyhow!("file has no extension"))?,
        &std::fs::read(&file)?,
        occluview_render::ThumbnailSpec {
            size_px: size,
            ..Default::default()
        },
    )
    .map_err(|e| anyhow!("render failed: {e}"))?;

    eprintln!("Writing {}...", out_path.display());
    let img = image::RgbaImage::from_raw(u32::from(size), u32::from(size), pixels)
        .ok_or_else(|| anyhow!("failed to create image buffer"))?;
    img.save(&out_path)
        .with_context(|| format!("writing {}", out_path.display()))?;

    eprintln!("Done: {}", out_path.display());
    Ok(())
}

/// `info <file>` - print mesh statistics.
fn cmd_info(args: &mut impl Iterator<Item = String>) -> Result<()> {
    let file: PathBuf = args
        .next()
        .ok_or_else(|| anyhow!("info: missing <file> argument"))?
        .into();
    let mut mesh = read_file(&file).with_context(|| format!("loading {}", file.display()))?;

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
         info      <file>                          Print format / counts / bbox\n    \
         help                                       Show this message"
    );
}
