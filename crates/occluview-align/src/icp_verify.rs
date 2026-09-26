//! Independent verification of a finished pose.
//!
//! The solver's own statistics describe the correspondences it chose to keep:
//! the closest `matching_ratio` share, a ratio the operator can lower to 0.1
//! and the global search lowers on its own. Judged on that share, a pose is
//! only asked whether its best tenth fits, and the other jaw, another
//! patient's arch, or a patch seated one tooth over all answered yes.
//!
//! This pass asks the question again at the final pose with nothing trimmed:
//! every sampled moving vertex looks for fixed surface within the operator's
//! reach, a vertex whose nearest fixed point is the fixed scan's open border
//! has no counterpart and is left out (the boundary rejection of Turk & Levoy,
//! "Zippered polygon meshes", SIGGRAPH 1994), and the answer is
//!
//! * how much of the moving surface found a counterpart,
//! * the median distance to it, which a true seating holds at the scanners'
//!   own agreement and a wrong one cannot, and
//! * whether the part that seats tightly pins the pose down by itself: the
//!   smallest eigenvalue of its point-to-plane information matrix, rotations
//!   scaled by the patch's own radius so the six directions are comparable
//!   (the stability analysis of Gelfand et al., "Geometrically stable sampling
//!   for the ICP algorithm", 3DIM 2003). A patch that seats perfectly but could
//!   still slide or turn — a smooth crest, a tooth turned about its own axis
//!   with its cusps off — has a near-zero eigenvalue there.

use glam::DVec3;
use rayon::prelude::*;

use crate::sample::{sample_vertices, vertex_at};
use crate::{Rigid, Soup, SurfaceIndex};

use super::{symmetric_eigendecomposition, Orientation, RefineSettings, DENSE_BUDGET};

/// A correspondence closer than this counts as tightly seated when judging
/// whether the seated part alone determines the pose, in millimetres: a few
/// times the agreement of two acquisitions of the same surface.
const TIGHT_BAND_MM: f64 = 0.1;

/// Fewer tightly seated samples than this determine nothing.
const MIN_TIGHT_SAMPLES: usize = 64;

/// What the verification pass measured at one pose.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Verification {
    /// Sampled moving vertices with a counterpart, over all sampled.
    pub(crate) coverage: f64,
    /// Median distance from those vertices to their counterpart, in mm.
    pub(crate) median_mm: f64,
    /// Smallest eigenvalue of the tightly seated part's normalized
    /// information matrix; zero when that part is too small to count.
    pub(crate) stability: f64,
}

impl Verification {
    /// The verification of a pose nothing was measured at.
    pub(crate) const NONE: Self = Self {
        coverage: 0.0,
        median_mm: f64::INFINITY,
        stability: 0.0,
    };
}

/// Verify `pose` of `moving` against `fixed` under the operator's reach and
/// orientation rule. `normals` are the moving vertex normals in its own frame.
pub(crate) fn verify(
    moving: Soup<'_>,
    normals: &[DVec3],
    fixed: &SurfaceIndex,
    pose: Rigid,
    settings: &RefineSettings,
) -> Verification {
    verify_with_budget(moving, normals, fixed, pose, settings, DENSE_BUDGET)
}

/// [`verify`] over at most `budget` sampled vertices.
pub(crate) fn verify_with_budget(
    moving: Soup<'_>,
    normals: &[DVec3],
    fixed: &SurfaceIndex,
    pose: Rigid,
    settings: &RefineSettings,
    budget: usize,
) -> Verification {
    let samples = sample_vertices(moving, budget);
    if samples.is_empty() || !pose.is_finite() {
        return Verification::NONE;
    }
    let reach = settings.influence_radius_mm.abs();
    let found: Vec<(f64, DVec3, DVec3)> = samples
        .par_iter()
        .filter_map(|&raw| {
            let vertex = raw as usize;
            let point = pose.apply(vertex_at(moving.positions, vertex)?);
            let hit = fixed.nearest(point, reach)?;
            if hit.on_border {
                return None;
            }
            let agreement = pose
                .apply_normal(normals.get(vertex).copied()?)
                .dot(hit.pseudo_normal);
            let facing = match settings.orientation {
                Orientation::Match => agreement > 0.0,
                Orientation::Inverted => agreement < 0.0,
                Orientation::Ignored => true,
            };
            facing.then(|| ((point - hit.point).length(), hit.point, hit.normal))
        })
        .collect();
    if found.is_empty() {
        return Verification::NONE;
    }
    let mut distances: Vec<f64> = found.iter().map(|entry| entry.0).collect();
    distances.sort_by(f64::total_cmp);
    #[allow(clippy::cast_precision_loss)]
    let coverage = found.len() as f64 / samples.len() as f64;
    Verification {
        coverage,
        median_mm: distances[distances.len() / 2],
        stability: stability(found.iter().filter(|entry| entry.0 <= TIGHT_BAND_MM)),
    }
}

/// Smallest eigenvalue of the normalized point-to-plane information matrix of
/// the given `(distance, target, normal)` correspondences, rotations scaled by
/// their RMS radius about their own centroid.
fn stability<'a>(seated: impl Iterator<Item = &'a (f64, DVec3, DVec3)> + Clone) -> f64 {
    let count = seated.clone().count();
    if count < MIN_TIGHT_SAMPLES {
        return 0.0;
    }
    #[allow(clippy::cast_precision_loss)]
    let weight = 1.0 / count as f64;
    let centre = seated.clone().fold(DVec3::ZERO, |sum, entry| sum + entry.1) * weight;
    let radius = (seated
        .clone()
        .map(|entry| (entry.1 - centre).length_squared())
        .sum::<f64>()
        * weight)
        .sqrt();
    if !(radius.is_finite() && radius > 0.0) {
        return 0.0;
    }
    let mut matrix = [[0.0_f64; 6]; 6];
    for entry in seated {
        let turn = (entry.1 - centre).cross(entry.2) / radius;
        let row = [turn.x, turn.y, turn.z, entry.2.x, entry.2.y, entry.2.z];
        for (i, line) in matrix.iter_mut().enumerate() {
            for (j, cell) in line.iter_mut().enumerate() {
                *cell += row[i] * row[j] * weight;
            }
        }
    }
    let (eigenvalues, _) = symmetric_eigendecomposition(matrix);
    // Fail closed: a non-finite eigenvalue determines nothing.
    if eigenvalues.iter().any(|value| !value.is_finite()) {
        return 0.0;
    }
    eigenvalues
        .into_iter()
        .fold(f64::INFINITY, f64::min)
        .max(0.0)
}
