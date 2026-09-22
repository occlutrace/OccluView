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
use occluview_formats::dispatch::{
    read_file_loaded_with_key_provider, read_files_with_key_provider,
};
use occluview_formats::hps::RuntimeHpsKeyProvider;
use std::ffi::{OsStr, OsString};
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const MAX_CLI_THUMBNAIL_SIZE: u16 = 4096;

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
    let mut args = std::env::args_os().skip(1);
    let subcommand = args.next().unwrap_or_else(|| {
        print_usage_with_error();
        OsString::from("help")
    });

    match subcommand.to_str() {
        Some("thumbnail") => cmd_thumbnail(&mut args),
        Some("convert") => cmd_convert(&mut args),
        Some("close-holes") => cmd_close_holes(&mut args),
        Some("info") => cmd_info(&mut args),
        Some("help" | "--help" | "-h") => {
            print_usage();
            Ok(())
        }
        Some("--version" | "-V") => {
            println!("occluview-cli {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some(other) => {
            print_usage_with_error();
            Err(anyhow!("unknown subcommand: {other}"))
        }
        None => {
            print_usage_with_error();
            Err(anyhow!("subcommand is not valid UTF-8"))
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
    args: &mut impl Iterator<Item = OsString>,
    subcommand: &str,
) -> Result<FileArgument> {
    let first = args
        .next()
        .ok_or_else(|| anyhow!("{subcommand}: missing <file> argument"))?;
    match first.to_str() {
        Some("-h" | "--help") => Ok(FileArgument::Help),
        Some(flag) if flag.starts_with('-') => Err(anyhow!(
            "{subcommand}: expected a file path, got the flag {flag}; the file comes first"
        )),
        Some(_) | None => Ok(FileArgument::Path(PathBuf::from(first))),
    }
}

fn take_path_argument(args: &mut impl Iterator<Item = OsString>, option: &str) -> Result<PathBuf> {
    args.next()
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("{option} requires a path"))
}

fn take_utf8_argument(args: &mut impl Iterator<Item = OsString>, option: &str) -> Result<String> {
    let value = args
        .next()
        .ok_or_else(|| anyhow!("{option} requires a value"))?;
    value
        .into_string()
        .map_err(|_| anyhow!("{option} value is not valid UTF-8"))
}

fn format_cli_flag(arg: &OsStr) -> String {
    arg.to_string_lossy().into_owned()
}

fn validate_thumbnail_size(raw: &str) -> Result<u16> {
    let size: u32 = raw.parse().context("--size must be a number")?;
    if !(1..=u32::from(MAX_CLI_THUMBNAIL_SIZE)).contains(&size) {
        return Err(anyhow!(
            "--size must be between 1 and {MAX_CLI_THUMBNAIL_SIZE} pixels"
        ));
    }
    u16::try_from(size).map_err(|_| anyhow!("--size exceeds the supported range"))
}

fn parse_limit_mm(raw: &str) -> Result<f32> {
    let limit: f32 = raw.parse().context("--limit-mm must be a number")?;
    if !limit.is_finite() || limit < 0.0 {
        return Err(anyhow!("--limit-mm must be a finite non-negative number"));
    }
    Ok(limit)
}

fn normalize_thumbnail_output_path(path: PathBuf) -> Result<PathBuf> {
    let path = export::normalize_output_path(path);
    let Some(extension) = path.extension().and_then(|extension| extension.to_str()) else {
        return Err(anyhow!("thumbnail output must end in .png"));
    };
    if !extension.eq_ignore_ascii_case("png") {
        return Err(anyhow!(
            "thumbnail output must end in .png; got .{extension}"
        ));
    }
    Ok(path)
}

/// `thumbnail <file> [-o out.png] [--size N]`
fn cmd_thumbnail(args: &mut impl Iterator<Item = OsString>) -> Result<()> {
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
        match arg.to_str() {
            Some("-o" | "--output") => {
                output = Some(take_path_argument(args, "-o")?);
            }
            Some("--size") => {
                size = validate_thumbnail_size(&take_utf8_argument(args, "--size")?)?;
            }
            Some(other) => return Err(anyhow!("unknown flag: {other}")),
            None => return Err(anyhow!("unknown non-UTF-8 flag")),
        }
    }

    let out_path = normalize_thumbnail_output_path(match output {
        Some(path) => path,
        None => implicit_thumbnail_path(&file),
    })?;

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
    write_thumbnail_atomically(&out_path, &img)
        .with_context(|| format!("writing {}", out_path.display()))?;

    eprintln!("Done: {}", out_path.display());
    std::process::exit(0);
}

static NEXT_THUMBNAIL_TEMP_ID: AtomicU64 = AtomicU64::new(0);

fn write_thumbnail_atomically(path: &Path, image: &image::RgbaImage) -> Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .unwrap_or_else(|| OsStr::new("thumbnail.png"));
    let (temporary, file) = reserve_thumbnail_temp(parent, file_name)?;
    let result = (|| -> Result<()> {
        let mut writer = BufWriter::new(file);
        image.write_to(&mut writer, image::ImageFormat::Png)?;
        writer.flush()?;
        writer
            .into_inner()
            .map_err(std::io::IntoInnerError::into_error)?
            .sync_all()?;
        Ok(())
    })();
    if let Err(error) = result {
        let _ = std::fs::remove_file(&temporary);
        return Err(error);
    }
    if let Err(error) = replace_thumbnail_file(&temporary, path) {
        let _ = std::fs::remove_file(&temporary);
        return Err(error.into());
    }
    Ok(())
}

fn reserve_thumbnail_temp(parent: &Path, file_name: &OsStr) -> Result<(PathBuf, File)> {
    for _ in 0..16 {
        let id = NEXT_THUMBNAIL_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let mut temporary_name = OsString::from(".");
        temporary_name.push(file_name);
        temporary_name.push(format!(".occluview-{id}.tmp.png"));
        let temporary = parent.join(temporary_name);
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
        {
            Ok(file) => return Ok((temporary, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
    }
    Err(anyhow!("could not reserve a temporary thumbnail path"))
}

#[cfg(not(windows))]
fn replace_thumbnail_file(temporary: &Path, destination: &Path) -> std::io::Result<()> {
    std::fs::rename(temporary, destination)
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn replace_thumbnail_file(temporary: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let temporary: Vec<u16> = temporary
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        MoveFileExW(
            PCWSTR(temporary.as_ptr()),
            PCWSTR(destination.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    }
    .map_err(|error| std::io::Error::other(error.to_string()))
}

/// Where a thumbnail goes when the operator named no output.
///
/// `<scan>.png` is the name a PLY or OBJ names as its own texture, so writing a
/// thumbnail there would replace the scan's image with a picture of the scan.
/// An occupied name is therefore stepped aside, and only an explicit `-o`
/// replaces a file the operator chose by name.
fn implicit_thumbnail_path(file: &Path) -> PathBuf {
    let mut path = file.to_path_buf();
    path.set_extension("png");
    // The writer collapses a repeated terminal extension (`scan.png.stl` ->
    // `scan.png`), so the name to check is the one that will be written, not
    // the one before that collapse.
    path = export::normalize_output_path(path);
    if !path.exists() {
        return path;
    }
    let stem = path.file_stem().map_or_else(
        || "thumbnail".to_string(),
        |stem| stem.to_string_lossy().into_owned(),
    );
    path.set_file_name(format!("{stem}-thumb.png"));
    path
}

/// `convert <file> -o output.{stl|ply|obj}`
fn cmd_convert(args: &mut impl Iterator<Item = OsString>) -> Result<()> {
    let input: PathBuf = match take_file_argument(args, "convert")? {
        FileArgument::Help => {
            print_usage();
            return Ok(());
        }
        FileArgument::Path(path) => path,
    };
    let mut output: Option<PathBuf> = None;
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("-o" | "--output") => {
                output = Some(take_path_argument(args, "-o")?);
            }
            Some(other) => return Err(anyhow!("unknown flag: {other}")),
            None => return Err(anyhow!("unknown non-UTF-8 flag")),
        }
    }

    let output = export::normalize_output_path(
        output.ok_or_else(|| anyhow!("convert: missing -o <output-path>"))?,
    );
    let (format, report) = export::convert_file(&input, &output)?;
    export::print_write_warnings(&report);
    eprintln!(
        "Converted {} -> {} ({format:?})",
        input.display(),
        output.display()
    );
    Ok(())
}

/// `close-holes <file> -o out.stl [--limit-mm N]` - run Close Holes headlessly
/// and print the resulting edit report.
fn cmd_close_holes(args: &mut impl Iterator<Item = OsString>) -> Result<()> {
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
        match arg.to_str() {
            Some("-o" | "--output") => {
                output = Some(take_path_argument(args, "-o")?);
            }
            Some("--limit-mm") => {
                limit_mm = Some(parse_limit_mm(&take_utf8_argument(args, "--limit-mm")?)?);
            }
            Some(other) => return Err(anyhow!("unknown flag: {other}")),
            None => return Err(anyhow!("unknown non-UTF-8 flag")),
        }
    }
    let output = export::normalize_output_path(
        output.ok_or_else(|| anyhow!("close-holes: missing -o <output-path>"))?,
    );

    let (report, write_report) = export::close_holes_file(&input, &output, limit_mm)?;
    export::print_write_warnings(&write_report);
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
fn cmd_info(args: &mut impl Iterator<Item = OsString>) -> Result<()> {
    let raw: Vec<OsString> = args.collect();
    if raw
        .iter()
        .any(|arg| matches!(arg.to_str(), Some("-h" | "--help")))
    {
        print_usage();
        return Ok(());
    }
    if let Some(flag) = raw
        .iter()
        .find(|arg| arg.to_str().is_some_and(|value| value.starts_with('-')))
    {
        return Err(anyhow!(
            "info: expected file paths, got the flag {}; the files come first",
            format_cli_flag(flag)
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
            "[{}/{}] {}  verts={} tris={} kind={} units=[{}] bbox={:.1}x{:.1}x{:.1}mm",
            i + 1,
            scene.meshes().len(),
            files
                .get(i)
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
            m.vertices().len(),
            m.triangle_count(),
            if m.is_point_cloud() { "cloud" } else { "mesh" },
            entry.import_units(),
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

/// Single-file info (the original output format, plus a Units line).
fn cmd_info_one(file: &Path) -> Result<()> {
    let loaded = read_file_loaded_with_key_provider(file, &RuntimeHpsKeyProvider)
        .with_context(|| format!("loading {}", file.display()))?;
    let mesh = &loaded.mesh;

    let bbox = mesh.bbox();
    let [w, h, d] = bbox.dimensions_mm();

    println!("File:       {}", file.display());
    println!(
        "Format:     {}",
        file.extension().and_then(|e| e.to_str()).unwrap_or("?")
    );
    println!("Units:      {}", loaded.units);
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

    use super::{
        normalize_thumbnail_output_path, parse_limit_mm, take_file_argument,
        validate_thumbnail_size, write_thumbnail_atomically, FileArgument,
    };
    use std::ffi::OsString;
    use std::path::PathBuf;

    #[test]
    fn a_flag_where_the_file_belongs_is_refused_instead_of_opened() {
        let mut args = ["-o", "out.png"].into_iter().map(OsString::from);
        let error = take_file_argument(&mut args, "thumbnail")
            .expect_err("a flag must not be accepted as the file to render");
        let message = error.to_string();
        assert!(message.contains("-o"), "{message}");
        assert!(message.contains("thumbnail"), "{message}");
    }

    #[test]
    fn asking_a_subcommand_for_help_prints_help_and_renders_nothing() {
        for flag in ["-h", "--help"] {
            let mut args = std::iter::once(OsString::from(flag));
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
        let mut args = std::iter::once(OsString::from("scan.stl"));
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
            !thumbnail.contains("use_software_renderer_only"),
            "CLI rendering uses the same per-request verified adapter policy as Explorer"
        );
        assert!(
            thumbnail.contains("write_thumbnail_atomically"),
            "thumbnail output must be published only after the complete PNG is encoded"
        );
    }

    #[test]
    fn convert_cli_routes_through_export_module() {
        let source = production_source();
        assert!(source.contains("Some(\"convert\") => cmd_convert(&mut args)"));
        assert!(source.contains("export::convert_file(&input, &output)?;"));
        assert!(source.contains("output.{stl|ply|obj}"));
    }

    #[test]
    fn thumbnail_size_has_a_bounded_allocation_contract() {
        assert_eq!(validate_thumbnail_size("1").expect("minimum"), 1);
        assert_eq!(validate_thumbnail_size("4096").expect("maximum"), 4096);
        assert!(validate_thumbnail_size("0").is_err());
        assert!(validate_thumbnail_size("4097").is_err());
        assert!(validate_thumbnail_size("65535").is_err());
    }

    #[test]
    fn close_holes_limit_rejects_non_finite_and_negative_values() {
        assert!(
            (parse_limit_mm("0").expect("zero is a valid hard limit") - 0.0).abs() <= f32::EPSILON
        );
        assert!((parse_limit_mm("15.5").expect("finite limit") - 15.5).abs() <= f32::EPSILON);
        assert!(parse_limit_mm("-1").is_err());
        assert!(parse_limit_mm("NaN").is_err());
        assert!(parse_limit_mm("inf").is_err());
    }

    #[test]
    fn thumbnail_output_is_png_and_does_not_duplicate_its_extension() {
        assert_eq!(
            normalize_thumbnail_output_path(PathBuf::from("scan.png.png")).expect("png"),
            PathBuf::from("scan.png")
        );
        assert!(normalize_thumbnail_output_path(PathBuf::from("scan.jpg")).is_err());
        assert!(normalize_thumbnail_output_path(PathBuf::from("scan")).is_err());
    }

    #[test]
    fn thumbnail_overwrite_publishes_a_complete_png_without_a_temp_sibling() {
        let directory = tempfile::tempdir().expect("temp directory");
        let destination = directory.path().join("scan.png");
        std::fs::write(&destination, b"previous thumbnail").expect("seed thumbnail");
        let image = image::RgbaImage::from_pixel(3, 2, image::Rgba([12, 34, 56, 255]));

        write_thumbnail_atomically(&destination, &image).expect("publish thumbnail");

        let decoded = image::open(&destination).expect("decode published thumbnail");
        assert_eq!(decoded.width(), 3);
        assert_eq!(decoded.height(), 2);
        assert!(std::fs::read_dir(directory.path())
            .expect("read directory")
            .filter_map(Result::ok)
            .all(|entry| !entry.file_name().to_string_lossy().contains(".occluview-")));
    }
}
