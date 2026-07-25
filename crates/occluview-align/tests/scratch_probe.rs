//! TEMPORARY diagnostic probe. Delete before committing.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::print_stderr)]

use std::path::Path;

use glam::{DQuat, DVec3};
use occluview_align::{
    deviation, deviation_colors, deviation_stats, ramp_color, CancelFlag, DeviationSettings,
    RampMode, RampSettings, Rigid, Soup, SurfaceIndex,
};

const DIR: &str =
    "/tmp/claude-1101/-home-wow-occlutraceio/4e21c36a-f8d7-487e-89e0-33dc0df28bdb/scratchpad/scans";

#[test]
fn probe() {
    let Ok(bytes) = std::fs::read(Path::new(DIR).join("waxup.stl")) else {
        eprintln!("no fixture");
        return;
    };
    let (positions, indices) = read_binary_stl(&bytes);
    eprintln!(
        "waxup: {} verts {} tris",
        positions.len() / 3,
        indices.len() / 3
    );

    let soup = Soup {
        positions: &positions,
        indices: &indices,
        mask: None,
    };
    let index = SurfaceIndex::build(soup).unwrap();

    for shift in [0.0_f64, 0.05, 0.3, 1.0, 3.0] {
        let pose = Rigid::new(DQuat::IDENTITY, DVec3::new(0.0, 0.0, shift));
        let settings = DeviationSettings {
            influence_radius_mm: 5.0,
            orientation: occluview_align::Orientation::Match,
        };
        let map = deviation(soup, &index, pose, &settings, &CancelFlag::new());
        let stats = deviation_stats(&map, 0.2);
        let ramp = RampSettings {
            scale_mm: 0.5,
            tolerance_mm: 0.2,
            bands: None,
            mode: RampMode::Magnitude,
        };
        let colors = deviation_colors(&map, &ramp);
        let mut blue = 0usize;
        let mut cyan = 0usize;
        let mut green = 0usize;
        let mut yellow = 0usize;
        let mut red = 0usize;
        let mut grey = 0usize;
        for c in &colors {
            if *c == occluview_align::NO_DATA_COLOR {
                grey += 1;
            } else if c[0] == 0 && c[1] < 150 {
                blue += 1;
            } else if c[0] == 0 {
                cyan += 1;
            } else if c[0] < 150 {
                green += 1;
            } else if c[1] > 120 {
                yellow += 1;
            } else {
                red += 1;
            }
        }
        eprintln!(
            "shift {shift:>4} mm: mean_abs {:.4} rms {:.4} p95 {:.4} measured {} skipped {} | blue {blue} cyan {cyan} green {green} yellow {yellow} red {red} grey {grey}",
            stats.mean_abs, stats.rms, stats.p95, stats.measured, stats.skipped
        );
    }
}

#[test]
fn ramp_walk() {
    let ramp = RampSettings::default();
    for step in 0..=20 {
        let position = f64::from(step) / 20.0;
        let c = ramp_color(position * ramp.scale_mm, &ramp);
        eprintln!("pos {position:.2} -> {c:?}");
    }
}

#[test]
fn legend_walk() {
    let ramp = RampSettings::default();
    eprintln!("--- legend as painted today (-1..+1) ---");
    for step in 0..8 {
        let position = (f64::from(step) / 7.0).mul_add(2.0, -1.0);
        let c = ramp_color(position * ramp.scale_mm, &ramp);
        eprintln!("legend t {position:+.2} -> {c:?}");
    }
}

fn read_binary_stl(bytes: &[u8]) -> (Vec<f32>, Vec<u32>) {
    let count = u32::from_le_bytes([bytes[80], bytes[81], bytes[82], bytes[83]]) as usize;
    let mut positions = Vec::with_capacity(count * 9);
    let mut indices = Vec::with_capacity(count * 3);
    for triangle in 0..count {
        let base = 84 + triangle * 50;
        if base + 50 > bytes.len() {
            break;
        }
        for corner in 0..3 {
            for axis in 0..3 {
                let at = base + 12 + corner * 12 + axis * 4;
                positions.push(f32::from_le_bytes([
                    bytes[at],
                    bytes[at + 1],
                    bytes[at + 2],
                    bytes[at + 3],
                ]));
            }
        }
        let first = u32::try_from(triangle * 3).unwrap();
        indices.extend_from_slice(&[first, first + 1, first + 2]);
    }
    (positions, indices)
}

#[test]
fn real_pair() {
    let load = |name: &str| -> Option<(Vec<f32>, Vec<u32>)> {
        let bytes = std::fs::read(Path::new(DIR).join(name)).ok()?;
        Some(read_binary_stl(&bytes))
    };
    let Some((wax_p, wax_i)) = load("waxup.stl") else {
        return;
    };
    let Some((hyb_p, hyb_i)) = load("hybrid_with_adaptaion.stl") else {
        return;
    };
    eprintln!(
        "waxup {} verts, hybrid {} verts",
        wax_p.len() / 3,
        hyb_p.len() / 3
    );
    eprintln!("identical bytes? {}", wax_p == hyb_p);

    let fixed = Soup {
        positions: &hyb_p,
        indices: &hyb_i,
        mask: None,
    };
    let moving = Soup {
        positions: &wax_p,
        indices: &wax_i,
        mask: None,
    };
    let index = SurfaceIndex::build(fixed).unwrap();
    let settings = DeviationSettings {
        influence_radius_mm: 5.0,
        orientation: occluview_align::Orientation::Match,
    };
    let map = deviation(
        moving,
        &index,
        Rigid::IDENTITY,
        &settings,
        &CancelFlag::new(),
    );
    let stats = deviation_stats(&map, 0.2);
    eprintln!(
        "PAIR: mean_abs {:.4} rms {:.4} p95 {:.4} median {:.4} measured {} skipped {}",
        stats.mean_abs, stats.rms, stats.p95, stats.median, stats.measured, stats.skipped
    );
    let mut hist = [0usize; 12];
    for (v, s) in map.signed_mm.iter().zip(&map.validity) {
        if *s != occluview_align::Validity::Measured {
            hist[11] += 1;
            continue;
        }
        let a = f64::from(v.abs());
        let bucket = ((a / 0.1).floor() as usize).min(10);
        hist[bucket] += 1;
    }
    for (i, n) in hist.iter().enumerate().take(11) {
        eprintln!(
            "  |dev| {:.1}-{:.1} mm: {n}",
            i as f64 * 0.1,
            (i + 1) as f64 * 0.1
        );
    }
    eprintln!("  unmeasured: {}", hist[11]);
}
