//! Temporary measurement harness: a folder of scans as Explorer would ask for it.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::print_stdout)]

use occluview_thumbnail::render_thumb::ThumbnailAttempt;
use occluview_thumbnail::render_thumb::DEFAULT_THUMBNAIL_TIMEOUT;
use occluview_thumbnail::{placeholder_thumbnail, try_render_thumbnail_file, ThumbnailSpec};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

fn main() {
    let mut args = std::env::args().skip(1);
    let dir = PathBuf::from(args.next().expect("usage: bulk_probe <dir> <threads>"));
    let threads: usize = args.next().unwrap_or_else(|| "12".into()).parse().unwrap();

    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("read_dir")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file())
        .collect();
    files.sort();
    println!("files={} threads={}", files.len(), threads);

    let spec = ThumbnailSpec {
        size_px: 256,
        ..ThumbnailSpec::default()
    };
    let placeholder = placeholder_thumbnail(spec);
    let queue = Arc::new(Mutex::new(files.clone()));
    let next = Arc::new(AtomicUsize::new(0));
    let results: Arc<Mutex<Vec<(u128, &'static str, PathBuf)>>> = Arc::new(Mutex::new(Vec::new()));

    let started = Instant::now();
    let mut handles = Vec::new();
    for _ in 0..threads {
        let queue = Arc::clone(&queue);
        let next = Arc::clone(&next);
        let results = Arc::clone(&results);
        let placeholder = placeholder.clone();
        handles.push(std::thread::spawn(move || loop {
            let index = next.fetch_add(1, Ordering::SeqCst);
            let path = {
                let queue = queue.lock().unwrap();
                match queue.get(index) {
                    Some(path) => path.clone(),
                    None => break,
                }
            };
            let at = Instant::now();
            let outcome = match try_render_thumbnail_file(&path, spec, DEFAULT_THUMBNAIL_TIMEOUT) {
                ThumbnailAttempt::Bitmap(pixels) if pixels == placeholder => "placeholder",
                ThumbnailAttempt::Bitmap(_) => "bitmap",
                ThumbnailAttempt::TransientFailure => "transient",
            };
            results
                .lock()
                .unwrap()
                .push((at.elapsed().as_millis(), outcome, path));
        }));
    }
    for handle in handles {
        handle.join().expect("worker panicked");
    }
    let wall = started.elapsed();

    let mut results = results.lock().unwrap().clone();
    results.sort_by_key(|(ms, _, _)| *ms);
    let count = results.len().max(1);
    let pick = |q: f64| results[((count as f64 - 1.0) * q) as usize].0;
    let bitmaps = results.iter().filter(|(_, o, _)| *o == "bitmap").count();
    let placeholders = results
        .iter()
        .filter(|(_, o, _)| *o == "placeholder")
        .count();
    let transients = results.iter().filter(|(_, o, _)| *o == "transient").count();
    println!(
        "wall={:?} per_file_ms p50={} p90={} p99={} max={} | bitmap={} placeholder={} transient={}",
        wall,
        pick(0.5),
        pick(0.9),
        pick(0.99),
        results.last().map_or(0, |r| r.0),
        bitmaps,
        placeholders,
        transients
    );
    println!(
        "throughput={:.1} files/s",
        results.len() as f64 / wall.as_secs_f64()
    );
    for (ms, outcome, path) in results.iter().rev().take(8) {
        println!("  slowest {ms:>7} ms {outcome:<11} {}", path.display());
    }
    for (ms, outcome, path) in results.iter().filter(|(_, o, _)| *o != "bitmap").take(8) {
        println!("  failed  {ms:>7} ms {outcome:<11} {}", path.display());
    }
}
