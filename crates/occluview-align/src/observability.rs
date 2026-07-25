//! HONESTY: how much of a rigid displacement a nearest-point map can actually
//! see, and how much can hide behind the number it prints.
//!
//! # The problem
//!
//! A deviation map reports, per moving vertex, the distance to the nearest
//! point on the fixed surface. Move the two surfaces apart along the fixed
//! surface's own normal and the map reports the move in full. Slide them past
//! each other along a direction the surface is smooth in and the nearest point
//! slides with them: the material has moved, and the map says almost nothing.
//! On a cylinder displaced 0.30 mm along its own axis the mean reads 0.0075 mm;
//! on a sphere turned one degree about any diameter, 0.0008 mm against a true
//! 0.055 mm. Measuring the reverse direction as well does not help — the
//! surfaces really do coincide as point sets. Nothing computed *from surface
//! distance alone* can recover the offset, so the tool must say how blind it is
//! instead of pretending it is not.
//!
//! # The measure
//!
//! Take a small rigid perturbation of the fitted pose — a twist `(ω, v)` about
//! the centroid `c` of the measured points — which displaces the measured point
//! `pᵢ` by `δᵢ = ω × (pᵢ − c) + v`.
//!
//! * What the map **sees** is, to first order, `δᵢ · nᵢ` at each vertex, with
//!   `nᵢ` the fixed surface normal at the hit. Its mean square is `ξᵀCξ / N`
//!   with `ξ = (ω, v)` and `C = Σ aᵢaᵢᵀ`, `aᵢ = [(pᵢ − c) × nᵢ ; nᵢ]` — the
//!   familiar point-to-plane normal matrix.
//! * What **truly happened** is `|δᵢ|`. Its mean square is `ξᵀMξ / N` with
//!   `M = Σ GᵢᵀGᵢ`, `Gᵢ = [−[pᵢ − c]× | I]`.
//!
//! Their ratio is what the map converts truth into:
//!
//! ```text
//! sensitivity(ξ) = RMS(seen) / RMS(true) = sqrt( ξᵀCξ / ξᵀMξ )
//! ```
//!
//! It lies in `[0, 1]` because `(δ · n)² ≤ |δ|²` for a unit normal. Its six
//! extremes are the generalized eigenvalues of `(C, M)`, and the smallest names
//! the direction the pair of surfaces is blindest in. That is the whole
//! measure: `1.0` means a millimetre of displacement reads as a millimetre,
//! `0.5` means it reads as half, `0.0` means the surfaces slide freely and the
//! map cannot see the motion at all.
//!
//! # What it returns on real geometry
//!
//! Measured on the fixtures in `tests/measurement_truth.rs` and on real arch
//! scans, the six sensitivities come out as:
//!
//! | fixture | six sensitivities | reading |
//! |---|---|---|
//! | plane | `0 0 0 1 1 1` | blind to both in-plane slides and the in-plane turn |
//! | cylinder | `0 0 .61 .61 .71 .71` | blind to the axial slide and the axial turn |
//! | sphere | `0 0 0 .50 .50 .71` | blind to all three turns |
//! | full arch scan | `.45 .48 .55 .60 .67 .68` | no blind direction, but every direction reads about half |
//!
//! The arch row is the important one. A real dental scan has no *fully* blind
//! direction — its curvature and undercuts see to that — but it under-reports a
//! rigid displacement by roughly a factor of two in every direction at once,
//! which is exactly the 0.30 mm offset that measured 0.14 mm.
//!
//! # Where this comes from
//!
//! `C` is the point-to-plane normal matrix of Gelfand, Ikemoto, Rusinkiewicz
//! and Levoy, "Geometrically Stable Sampling for the ICP Algorithm", 3DIM 2003,
//! equations 3 to 5, including their `cᵢ = pᵢ × nᵢ`. Their reading of it is the
//! one used here: "The transformations for which this increase is comparatively
//! small correspond to directions where the input meshes can slide relative to
//! each other", and "If any of the eigenvalues are small, the corresponding
//! eigenvector defines a transformation that can move two meshes from their
//! optimum alignment with only a small increase in error." They use the
//! condition number of `C` to choose samples; this module instead divides by
//! `M` so the result comes out as a dimensionless fraction of a displacement
//! rather than as a ratio between two eigenvalues of mixed units.
//!
//! The second-order geometry behind the collapse is Pottmann and Hofer,
//! "Geometry of the Squared Distance Function to Curves and Surfaces" (2003),
//! propositions 2 and 3: in the principal frame at the footpoint the tangential
//! coordinates enter the squared distance only through curvature, so a
//! tangential slide `t` on a surface of radius `r` reappears as roughly
//! `t²/2r` — 5 µm for a 100 µm slide on a 1 mm cusp, and exactly nothing on a
//! flat abutment face.
//!
//! That a small residual after a fit says nothing about error at the feature
//! you care about is the same result Fitzpatrick reports for landmark
//! registration in "Fiducial registration error and target registration error
//! are uncorrelated", SPIE Medical Imaging 2009. This module is the attempt to
//! give the operator the missing half of that picture rather than to leave the
//! residual standing on its own.

use glam::DVec3;
use rayon::prelude::*;

use crate::deviation::DeviationSettings;
use crate::sample::{sample_vertices, vertex_at};
use crate::{CancelFlag, Rigid, Soup, SurfaceIndex};

/// Vertices sampled for the estimate. Six parameters do not need more, and the
/// stride spreads them over the whole surface.
const SAMPLE_BUDGET: usize = 40_000;

/// Fewer measured samples than this cannot determine six degrees of freedom
/// with any margin.
const MIN_SAMPLES: usize = 32;

/// Below this normal length a hit carries no usable direction.
const MIN_NORMAL_LENGTH: f64 = 1e-9;

/// A Cholesky pivot at or below this means the sampled points do not span the
/// six rigid degrees of freedom — a single point, or a line of them.
const MIN_PIVOT: f64 = 1e-12;

/// Cyclic Jacobi sweeps. A symmetric 6x6 converges in far fewer; the count is
/// fixed rather than tolerance-driven so the result cannot vary with rounding.
const JACOBI_SWEEPS: usize = 32;

/// A symmetric six-by-six, row-major.
type Matrix6 = [[f64; 6]; 6];

/// How much of a rigid displacement this pair of surfaces converts into
/// measured distance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Observability {
    /// The six rigid modes' sensitivities, smallest first, each in `[0, 1]`:
    /// millimetres of reported RMS deviation per millimetre of true RMS
    /// displacement. `1.0` is fully visible, `0.0` is invisible.
    pub sensitivity: [f64; 6],
    /// Rotation part of the least visible mode, in radians about [`Self::pivot`],
    /// scaled so the mode displaces the measured surface by 1 mm RMS.
    pub blind_rotation: DVec3,
    /// Translation part of the least visible mode, in millimetres, on the same
    /// 1 mm RMS scale.
    pub blind_translation: DVec3,
    /// Centroid of the measured points, in the fixed frame: the point
    /// [`Self::blind_rotation`] turns about.
    pub pivot: DVec3,
    /// Sampled vertices that carried a correspondence.
    pub samples: u32,
}

impl Observability {
    /// Fraction of a rigid displacement that reaches the number, in the
    /// direction this pair of surfaces is blindest in.
    #[must_use]
    pub fn worst_sensitivity(&self) -> f64 {
        self.sensitivity[0]
    }

    /// Fraction that reaches the number in the direction it sees best.
    #[must_use]
    pub fn best_sensitivity(&self) -> f64 {
        self.sensitivity[5]
    }

    /// The largest true rigid displacement, as an RMS in millimetres, that
    /// could be hiding behind a reported RMS deviation of `reported_rms_mm`.
    ///
    /// This is the number to print beside the measurement: what the surfaces
    /// could have moved by while still producing the map the operator is
    /// looking at. Infinite when a direction is entirely blind, and that is the
    /// correct answer — a free slide is unbounded.
    ///
    /// # Accuracy
    ///
    /// First order in the displacement, so it is exact in the limit and
    /// approximate at a finite offset: at a finite offset the nearest point
    /// lands on differently-oriented triangles than the ones the sensitivity
    /// was built from. Measured on real arch scans by displacing them along the
    /// blind mode by a known amount, this estimate came to 0.996 to 1.007 of
    /// the truth at 0.02 mm, and 0.94 to 0.98 of it at 0.30 mm. It can
    /// therefore understate by a few percent at clinical magnitudes. Treat it
    /// as a correction of the right size, not as a certified bound: it exists
    /// to undo a factor-of-two understatement, and a six percent error inside
    /// that correction does not change any decision the factor of two does.
    #[must_use]
    pub fn hidden_displacement_mm(&self, reported_rms_mm: f64) -> f64 {
        let worst = self.worst_sensitivity();
        if worst <= 0.0 || !reported_rms_mm.is_finite() {
            return f64::INFINITY;
        }
        reported_rms_mm / worst
    }

    /// Whether the pair is blind enough in some direction that a surface map
    /// alone cannot be trusted to bound the displacement.
    ///
    /// The threshold is the point at which the hidden displacement exceeds ten
    /// times the reported one — past that the number on screen says more about
    /// the shape of the surfaces than about how well they fit.
    #[must_use]
    pub fn has_blind_direction(&self) -> bool {
        self.worst_sensitivity() < 0.1
    }
}

/// Measure what the deviation map for this pair can and cannot see.
///
/// Returns `None` when too little of the moving mesh reaches the fixed surface,
/// or when the samples that do reach it do not span six degrees of freedom — a
/// single patch of a plane, or a line of points. Both are cases where no
/// sensitivity exists to report, and reporting one anyway would be the lie this
/// module exists to prevent.
///
/// Deterministic: the correspondence search is parallel because it is pure, and
/// the two matrices are folded **serially in sample order**, so the answer is
/// bit-identical across thread counts.
#[must_use]
pub fn observability(
    moving: Soup<'_>,
    fixed: &SurfaceIndex,
    pose: Rigid,
    settings: &DeviationSettings,
    cancel: &CancelFlag,
) -> Option<Observability> {
    let samples = sample_vertices(moving, SAMPLE_BUDGET);
    if samples.is_empty() || cancel.is_cancelled() {
        return None;
    }

    let hits: Vec<Option<(DVec3, DVec3)>> = samples
        .par_iter()
        .map(|&raw| {
            if cancel.is_cancelled() {
                return None;
            }
            let local = vertex_at(moving.positions, raw as usize)?;
            let point = pose.apply(local);
            let hit = fixed.nearest(point, settings.influence_radius_mm)?;
            if hit.normal.length() < MIN_NORMAL_LENGTH {
                return None;
            }
            Some((point, hit.normal.normalize()))
        })
        .collect();

    let mut pivot = DVec3::ZERO;
    let mut count = 0usize;
    for (point, _) in hits.iter().flatten() {
        pivot += *point;
        count += 1;
    }
    if count < MIN_SAMPLES {
        return None;
    }
    #[allow(clippy::cast_precision_loss)]
    let total = count as f64;
    pivot /= total;

    let (seen, truth) = accumulate(&hits, pivot);
    let lower = cholesky(&truth)?;
    let (values, vectors) = jacobi(&whiten(&lower, &seen));

    let mut order = [0usize; 6];
    for (slot, entry) in order.iter_mut().enumerate() {
        *entry = slot;
    }
    order.sort_by(|left, right| values[*left].total_cmp(&values[*right]));

    let mut sensitivity = [0.0f64; 6];
    for (slot, source) in order.iter().enumerate() {
        sensitivity[slot] = values[*source].max(0.0).sqrt().min(1.0);
    }

    // The eigenvector is unit in the whitened frame, where `ξᵀMξ = 1`, so the
    // mode it names displaces the surface by `sqrt(1/N)` mm RMS. Scaling by
    // `sqrt(N)` puts it on the 1 mm RMS scale the fields promise.
    let whitened: [f64; 6] = core::array::from_fn(|row| vectors[row][order[0]]);
    let twist = unwhiten(&lower, &whitened);
    let scale = total.sqrt();

    Some(Observability {
        sensitivity,
        blind_rotation: DVec3::new(twist[0], twist[1], twist[2]) * scale,
        blind_translation: DVec3::new(twist[3], twist[4], twist[5]) * scale,
        pivot,
        samples: u32::try_from(count).unwrap_or(u32::MAX),
    })
}

/// Fold the correspondences into `C` (what the map sees) and `M` (what moved).
///
/// Serial and in sample order: a parallel reduction over floating-point sums
/// gives a different last bit for a different thread count, and a sensitivity
/// that changes with the machine is not a measurement.
fn accumulate(hits: &[Option<(DVec3, DVec3)>], pivot: DVec3) -> (Matrix6, Matrix6) {
    let mut seen = [[0.0f64; 6]; 6];
    let mut truth = [[0.0f64; 6]; 6];
    for (point, normal) in hits.iter().flatten() {
        let radius = *point - pivot;
        let moment = radius.cross(*normal);
        let row = [moment.x, moment.y, moment.z, normal.x, normal.y, normal.z];
        for (index, target) in seen.iter_mut().enumerate() {
            for (slot, cell) in target.iter_mut().enumerate() {
                *cell += row[index] * row[slot];
            }
        }
        // `Gᵢ` maps a twist to a displacement: `δ = ω × r + v`, one row per
        // world axis.
        let jacobian = [
            [0.0, radius.z, -radius.y, 1.0, 0.0, 0.0],
            [-radius.z, 0.0, radius.x, 0.0, 1.0, 0.0],
            [radius.y, -radius.x, 0.0, 0.0, 0.0, 1.0],
        ];
        for axis in &jacobian {
            for (index, target) in truth.iter_mut().enumerate() {
                for (slot, cell) in target.iter_mut().enumerate() {
                    *cell += axis[index] * axis[slot];
                }
            }
        }
    }
    (seen, truth)
}

/// Lower-triangular `L` with `L Lᵀ = matrix`, or `None` when the matrix is not
/// positive definite — which here means the samples do not span six degrees of
/// freedom.
fn cholesky(matrix: &Matrix6) -> Option<Matrix6> {
    let mut lower = [[0.0f64; 6]; 6];
    for row in 0..6 {
        for col in 0..=row {
            let mut sum = matrix[row][col];
            for back in 0..col {
                sum -= lower[row][back] * lower[col][back];
            }
            if row == col {
                // `is_finite` first, so a NaN pivot is rejected rather than
                // slipping through a comparison that is false either way.
                if !sum.is_finite() || sum <= MIN_PIVOT {
                    return None;
                }
                lower[row][col] = sum.sqrt();
            } else {
                lower[row][col] = sum / lower[col][col];
            }
        }
    }
    Some(lower)
}

/// `L⁻¹ A L⁻ᵀ`, which turns the generalized problem `Cξ = λMξ` into an ordinary
/// symmetric one.
///
/// A triangular solve reads two matrices at transposed positions of the same
/// pair of indices, which is what the loops below say and what an iterator form
/// would hide.
#[allow(clippy::needless_range_loop)]
fn whiten(lower: &Matrix6, matrix: &Matrix6) -> Matrix6 {
    let mut work = *matrix;
    for col in 0..6 {
        for row in 0..6 {
            let mut sum = work[row][col];
            for back in 0..row {
                sum -= lower[row][back] * work[back][col];
            }
            work[row][col] = sum / lower[row][row];
        }
    }
    for row in 0..6 {
        for col in 0..6 {
            let mut sum = work[row][col];
            for back in 0..col {
                sum -= lower[col][back] * work[row][back];
            }
            work[row][col] = sum / lower[col][col];
        }
    }
    work
}

/// Carry a whitened vector back to a twist: solve `Lᵀ ξ = y`.
fn unwhiten(lower: &Matrix6, whitened: &[f64; 6]) -> [f64; 6] {
    let mut out = *whitened;
    for row in (0..6).rev() {
        let mut sum = out[row];
        for forward in (row + 1)..6 {
            sum -= lower[forward][row] * out[forward];
        }
        out[row] = sum / lower[row][row];
    }
    out
}

/// Cyclic Jacobi on a symmetric 6x6: eigenvalues on the diagonal, eigenvectors
/// in the columns.
fn jacobi(matrix: &Matrix6) -> ([f64; 6], Matrix6) {
    let mut work = *matrix;
    let mut vectors = [[0.0f64; 6]; 6];
    for (slot, row) in vectors.iter_mut().enumerate() {
        row[slot] = 1.0;
    }
    for _ in 0..JACOBI_SWEEPS {
        for p in 0..5 {
            for q in (p + 1)..6 {
                let off = work[p][q];
                if off == 0.0 {
                    continue;
                }
                let theta = (work[q][q] - work[p][p]) / (2.0 * off);
                let sign = if theta >= 0.0 { 1.0 } else { -1.0 };
                let tangent = sign / (theta.abs() + theta.mul_add(theta, 1.0).sqrt());
                let cosine = 1.0 / tangent.mul_add(tangent, 1.0).sqrt();
                let sine = tangent * cosine;
                for row in &mut work {
                    let (left, right) = (row[p], row[q]);
                    row[p] = cosine * left - sine * right;
                    row[q] = sine * left + cosine * right;
                }
                for slot in 0..6 {
                    let (left, right) = (work[p][slot], work[q][slot]);
                    work[p][slot] = cosine * left - sine * right;
                    work[q][slot] = sine * left + cosine * right;
                }
                for row in &mut vectors {
                    let (left, right) = (row[p], row[q]);
                    row[p] = cosine * left - sine * right;
                    row[q] = sine * left + cosine * right;
                }
            }
        }
    }
    (core::array::from_fn(|slot| work[slot][slot]), vectors)
}
