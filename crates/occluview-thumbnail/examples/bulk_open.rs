//! Temporary measurement harness: opening many scans at once, as the app does.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::print_stdout)]

use std::path::PathBuf;
use std::time::Instant;

fn peak_rss_mb() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .unwrap_or_default()
        .lines()
        .find(|l| l.starts_with("VmHWM:"))
        .and_then(|l| {
            l.split_whitespace()
                .nth(1)
                .and_then(|v| v.parse::<u64>().ok())
        })
        .map(|kb| kb / 1024)
        .unwrap_or(0)
}

fn main() {
    let mut args = std::env::args().skip(1);
    let dir = PathBuf::from(args.next().expect("usage: bulk_open <dir> <count>"));
    let count: usize = args.next().unwrap_or_else(|| "20".into()).parse().unwrap();
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("read_dir")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file())
        .collect();
    files.sort();
    files.truncate(count);
    let bytes: u64 = files
        .iter()
        .filter_map(|p| p.metadata().ok())
        .map(|m| m.len())
        .sum();
    println!(
        "opening {} files, {} MB on disk",
        files.len(),
        bytes / 1024 / 1024
    );

    let at = Instant::now();
    match occluview_formats::read_files(&files) {
        Ok(scene) => println!(
            "ok in {:?}, layers={}, peak RSS {} MB",
            at.elapsed(),
            scene.meshes().len(),
            peak_rss_mb()
        ),
        Err((path, error)) => println!(
            "FAILED after {:?} on {}: {error} (peak RSS {} MB)",
            at.elapsed(),
            path.display(),
            peak_rss_mb()
        ),
    }
}
