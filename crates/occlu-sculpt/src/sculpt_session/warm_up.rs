//! Exercise each brush path on a private surface before the first stroke.
//!
//! The private sphere session is dropped at the end. The caller's session is
//! untouched, and its diagnostic counters are restored to their input values.

use super::*;
use crate::hash::FxHashMap;

const SPHERE_RADIUS_MM: f64 = 4.0;
/// Three subdivisions of an icosahedron: 1 280 faces, edges near 0.5 mm, so
/// a brush of the size below both refines and merges.
const SPHERE_SUBDIVISIONS: usize = 3;
const BRUSH_RADIUS_MM: f64 = 1.0;
const BRUSH_STRENGTH: f64 = 0.5;
const RAY_DISTANCE_MM: f64 = 12.0;
/// Pointer steps per brush and their spacing along the sphere, radians. Four
/// steps give a press and three swept steps.
const STEPS_PER_BRUSH: usize = 4;
const STEP_ANGLE: f64 = 0.15;

/// Run one short stroke of every brush on a private sphere.
pub fn warm_up_brush_step() {
    let saved = diag::take();
    let (verts, tris) = icosphere(SPHERE_RADIUS_MM, SPHERE_SUBDIVISIONS);
    let mut session = SculptSession::new(verts, tris);
    let brushes = [
        (BrushMode::Smooth, TipStamp::Ball),
        (BrushMode::Relax, TipStamp::Ball),
        (BrushMode::Deposit, TipStamp::Ball),
        (BrushMode::Erode, TipStamp::Ball),
        (BrushMode::Flatten, TipStamp::Cylinder),
        (BrushMode::Deposit, TipStamp::Knife),
    ];
    for (index, (mode, tip)) in brushes.into_iter().enumerate() {
        // Each brush works fresh ground, so every stroke remeshes.
        let latitude = -0.6 + 0.3 * index as f64;
        let start = 0.9 * index as f64;
        session.set_brush_tip(tip);
        session.set_dab_elapsed_ms(DWELL_FULL_DOSE_MS);
        session.start_stroke();
        for step in 0..STEPS_PER_BRUSH {
            let longitude = start + STEP_ANGLE * step as f64;
            let outward = DVec3::new(
                latitude.cos() * longitude.cos(),
                latitude.sin(),
                latitude.cos() * longitude.sin(),
            );
            let _ = session.dab_at_ray(
                outward * RAY_DISTANCE_MM,
                outward * (-1.0),
                BRUSH_RADIUS_MM,
                BRUSH_STRENGTH,
                mode,
                false,
            );
        }
        let _ = session.end_stroke();
    }
    drop(session);
    diag::restore(saved);
}

/// A closed icosphere, wound outward, in the kernel's f32 vertex layout.
fn icosphere(radius: f64, subdivisions: usize) -> (Vec<f32>, Vec<u32>) {
    let t = f64::midpoint(1.0, 5.0f64.sqrt());
    let mut points: Vec<DVec3> = [
        (-1.0, t, 0.0),
        (1.0, t, 0.0),
        (-1.0, -t, 0.0),
        (1.0, -t, 0.0),
        (0.0, -1.0, t),
        (0.0, 1.0, t),
        (0.0, -1.0, -t),
        (0.0, 1.0, -t),
        (t, 0.0, -1.0),
        (t, 0.0, 1.0),
        (-t, 0.0, -1.0),
        (-t, 0.0, 1.0),
    ]
    .iter()
    .map(|&(x, y, z)| DVec3::new(x, y, z).normalize_or_zero())
    .collect();
    let mut faces: Vec<[u32; 3]> = vec![
        [0, 11, 5],
        [0, 5, 1],
        [0, 1, 7],
        [0, 7, 10],
        [0, 10, 11],
        [1, 5, 9],
        [5, 11, 4],
        [11, 10, 2],
        [10, 7, 6],
        [7, 1, 8],
        [3, 9, 4],
        [3, 4, 2],
        [3, 2, 6],
        [3, 6, 8],
        [3, 8, 9],
        [4, 9, 5],
        [2, 4, 11],
        [6, 2, 10],
        [8, 6, 7],
        [9, 8, 1],
    ];
    for _ in 0..subdivisions {
        let mut midpoints: FxHashMap<(u32, u32), u32> = FxHashMap::default();
        let mut midpoint = |points: &mut Vec<DVec3>, a: u32, b: u32| -> u32 {
            let key = if a < b { (a, b) } else { (b, a) };
            *midpoints.entry(key).or_insert_with(|| {
                let point = ((points[a as usize] + points[b as usize]) * 0.5).normalize_or_zero();
                points.push(point);
                (points.len() - 1) as u32
            })
        };
        let mut next = Vec::with_capacity(faces.len() * 4);
        for [a, b, c] in faces {
            let ab = midpoint(&mut points, a, b);
            let bc = midpoint(&mut points, b, c);
            let ca = midpoint(&mut points, c, a);
            next.extend([[a, ab, ca], [b, bc, ab], [c, ca, bc], [ab, bc, ca]]);
        }
        faces = next;
    }
    let verts = points
        .iter()
        .flat_map(|point| {
            let point = point * radius;
            [point.x as f32, point.y as f32, point.z as f32]
        })
        .collect();
    let tris = faces.into_iter().flatten().collect();
    (verts, tris)
}
