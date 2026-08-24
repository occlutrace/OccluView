//! `occluview-cli` - headless CLI.
//!
//! Subcommands:
//!   - `thumbnail <file> [-o out.png] [--size N]` - render a thumbnail.
//!   - `convert <file> -o out.{stl|ply|obj}` - transcode a mesh into a common
//!     exchange format. Keeps geometry, vertex colors (PLY/OBJ), normals, and
//!     UVs where the destination format supports them.
//!   - `info <file> [file...]` - print format, vertex/triangle counts, bbox,
//!     colors, UVs, texture. Multiple files print per-file stats + an
//!     aggregate scene bbox (upper+lower arch case).
//!   - `help` - show usage. `-h` / `--help` is accepted in place of a file.

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

/// Initialize CLI and thumbnailer diagnostics on stderr.
///
/// The default level is `warn`; `RUST_LOG` can enable more detail without
/// mixing diagnostics with command output.
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
        print_usage_with_error();
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
            print_usage_with_error();
            Err(anyhow!("unknown subcommand: {other}"))
        }
    }
}

/// The leading positional argument of a subcommand: the file to work on, or a
/// request for the usage text.
#[derive(Debug)]
enum FileArgument {
    Path(PathBuf),
    Help,
}

/// Parse the leading file argument without accepting a flag as a path.
/// Files whose names begin with `-` remain addressable through `./name`.
fn take_file_argument(
    args: &mut impl Iterator<Item = String>,
    subcommand: &str,
) -> Result<FileArgument> {
    let first = args
        .next()
        .ok_or_else(|| anyhow!("{subcommand}: missing <file> argument"))?;
    match first.as_str() {
        "-h" | "--help" => Ok(FileArgument::Help),
        flag if flag.starts_with('-') => Err(anyhow!(
            "{subcommand}: expected a file path, got the flag {flag}; the file comes first"
        )),
        _ => Ok(FileArgument::Path(PathBuf::from(first))),
    }
}

/// `thumbnail <file> [-o out.png] [--size N]`
fn cmd_thumbnail(args: &mut impl Iterator<Item = String>) -> Result<()> {
    let file: PathBuf = match take_file_argument(args, "thumbnail")? {
        FileArgument::Help => {
            print_usage();
            return Ok(());
        }
        FileArgument::Path(path) => path,
    };
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

    occluview_thumbnail::use_software_renderer_only();
    eprintln!("Rendering {size}x{size} thumbnail...");
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
    let input: PathBuf = match take_file_argument(args, "convert")? {
        FileArgument::Help => {
            print_usage();
            return Ok(());
        }
        FileArgument::Path(path) => path,
    };
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

/// `close-holes <file> -o out.stl [--limit-mm N]` - run Close Holes headlessly
/// and print the resulting edit report.
fn cmd_close_holes(args: &mut impl Iterator<Item = String>) -> Result<()> {
    let input: PathBuf = match take_file_argument(args, "close-holes")? {
        FileArgument::Help => {
            print_usage();
            return Ok(());
        }
        FileArgument::Path(path) => path,
    };
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
    let raw: Vec<String> = args.collect();
    if raw.iter().any(|arg| arg == "-h" || arg == "--help") {
        print_usage();
        return Ok(());
    }
    if let Some(flag) = raw.iter().find(|arg| arg.starts_with('-')) {
        return Err(anyhow!(
            "info: expected file paths, got the flag {flag}; the files come first"
        ));
    }
    let files: Vec<PathBuf> = raw.into_iter().map(PathBuf::from).collect();
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
    let mesh = read_file_with_key_provider(file, &RuntimeHpsKeyProvider)
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

/// Print requested usage text to stdout.
fn print_usage() {
    println!("{}", usage_text());
}

/// Print usage text alongside an error on stderr.
fn print_usage_with_error() {
    eprintln!("{}", usage_text());
}

fn usage_text() -> &'static str {
    concat!(
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
         --version | -V                             Print the version and exit\n\
         \n\
         Every subcommand takes its file first; -h or --help in that position\n\
         prints this message instead of naming a file."
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic)]

    /// The part of this file above the test module.
    ///
    /// Searching the whole of it matches the needle written in the assertion
    /// itself, so the guard would pass on its own text and the production line
    /// it names could be deleted with nothing going red.
    fn production_source() -> &'static str {
        let source = include_str!("main.rs");
        source
            .split_once("#[cfg(test)]\nmod tests")
            .map_or(source, |(production, _)| production)
    }

    use super::{take_file_argument, FileArgument};
    use std::path::PathBuf;

    #[test]
    fn a_flag_where_the_file_belongs_is_refused_instead_of_opened() {
        let mut args = ["-o", "out.png"].into_iter().map(String::from);
        let error = take_file_argument(&mut args, "thumbnail")
            .expect_err("a flag must not be accepted as the file to render");
        let message = error.to_string();
        assert!(message.contains("-o"), "{message}");
        assert!(message.contains("thumbnail"), "{message}");
    }

    #[test]
    fn asking_a_subcommand_for_help_prints_help_and_renders_nothing() {
        for flag in ["-h", "--help"] {
            let mut args = std::iter::once(flag.to_string());
            let taken =
                take_file_argument(&mut args, "thumbnail").expect("--help must not be an error");
            assert!(
                matches!(taken, FileArgument::Help),
                "{flag} must ask for usage, not name a file to render"
            );
        }
    }

    #[test]
    fn an_ordinary_path_is_still_taken_verbatim() {
        let mut args = std::iter::once("scan.stl".to_string());
        match take_file_argument(&mut args, "info").expect("a plain path is valid") {
            FileArgument::Path(path) => assert_eq!(path, PathBuf::from("scan.stl")),
            FileArgument::Help => panic!("a plain path is not a help request"),
        }
    }

    #[test]
    fn a_missing_file_is_reported_against_the_subcommand_that_wanted_it() {
        let mut args = std::iter::empty();
        let error =
            take_file_argument(&mut args, "close-holes").expect_err("close-holes needs a file");
        assert!(error.to_string().contains("close-holes"));
    }

    #[test]
    fn version_flag_is_recognised_and_advertised() {
        let source = production_source();
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
        let source = production_source();
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
        assert!(
            thumbnail.contains("occluview_thumbnail::use_software_renderer_only();"),
            "one render and then the process ends: a discrete driver's own threads \
             can fault while the process is torn down, and a file manager reads \
             that as a thumbnailer that failed"
        );
    }

    #[test]
    fn convert_cli_routes_through_export_module() {
        let source = production_source();
        assert!(source.contains("\"convert\" => cmd_convert(&mut args)"));
        assert!(source.contains("export::convert_file(&input, &output)?;"));
        assert!(source.contains("output.{stl|ply|obj}"));
    }
}
