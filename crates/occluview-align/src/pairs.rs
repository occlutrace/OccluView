//! ALIGN: the rigid fit from clicked surface point pairs.
//!
//! Three or more pairs use Horn's closed-form quaternion fit. Two pairs use
//! the clicked surface normals to build a frame per side, exactly as lab
//! software does — two bare points in space cannot determine a rotation, and
//! this module refuses rather than inventing one.
//!
//! Every refusal is a named [`FitRejection`] carrying what the caller needs to
//! tell the operator. A fit that cannot be determined is never returned as a
//! confident pose.

use glam::{DMat3, DQuat, DVec3};

use crate::Rigid;

/// The smallest number of pairs any fit accepts.
const MIN_PAIRS: usize = 2;

/// Below this fraction of the total spread, the points lie on a line and the
/// rotation about that line is free.
const COLLINEAR_FRACTION: f64 = 1e-6;

/// Distance ratio outside this band means the two sets are in different units
/// (millimetres against centimetres, say), which is a problem to report rather
/// than a scale to fit.
const UNIT_RATIO_LOW: f64 = 0.5;
/// Upper end of the accepted distance-ratio band.
const UNIT_RATIO_HIGH: f64 = 2.0;

/// A residual must exceed both this many times the median AND
/// [`TRIM_FLOOR_MM`] before trimming drops its pair.
const TRIM_MEDIAN_FACTOR: f64 = 3.0;

/// Absolute residual floor for trimming, in millimetres. Without it a clean
/// fit would trim forever: one 3e-16 residual genuinely is three times the
/// median of 1e-16.
const TRIM_FLOOR_MM: f64 = 0.05;

/// How nearly parallel a clicked normal may lie to the two-pair segment before
/// the frame it would build is meaningless.
const NORMAL_ALONG_SEGMENT_LIMIT: f64 = 0.99;

/// A world axis is named in a degeneracy report when the undetermined
/// direction has at least this much of its length along that axis. A unit
/// vector always clears it on at least one axis.
const WEAK_AXIS_SHARE: f64 = 0.5;

/// Cyclic Jacobi sweeps used to diagonalize the 4x4 Horn matrix, and power
/// iterations used to find the dominant spread direction. Both are fixed
/// counts rather than convergence-timed, so a result cannot drift between
/// runs.
const JACOBI_SWEEPS: usize = 24;
/// Power iterations for the dominant spread direction.
const POWER_ITERATIONS: usize = 32;

/// A successful fit and the diagnostics the UI reports alongside it.
#[derive(Clone, Debug, PartialEq)]
pub struct PairFit {
    /// The pose carrying the moving points onto the fixed ones.
    pub rigid: Rigid,
    /// Root-mean-square residual over the pairs that survived trimming, in mm.
    pub pair_rms: f64,
    /// Largest surviving residual, in mm.
    pub max_pair_err: f64,
    /// Indices of pairs dropped as outliers, ascending.
    pub rejected: Vec<u32>,
    /// Mean fixed distance over mean moving distance. One means same units.
    pub unit_ratio: f64,
}

/// Why a fit was refused. Each variant carries what the operator needs to act.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FitRejection {
    /// Fewer complete pairs than any fit can use.
    TooFewPairs {
        /// Complete pairs supplied.
        have: usize,
        /// Minimum this fit requires.
        need: usize,
    },
    /// The two point lists differ in length, so some clicked point has no
    /// partner. Refused rather than truncated: dropping a click silently would
    /// be worse than saying so.
    Unpaired {
        /// Points supplied on the moving side.
        moving: usize,
        /// Points supplied on the fixed side.
        fixed: usize,
    },
    /// The configuration does not determine a rotation.
    Degenerate {
        /// Per world axis, whether rotation about it is undetermined.
        weak_axes: [bool; 3],
    },
    /// The two sets are in different units.
    UnitMismatch {
        /// Mean fixed distance over mean moving distance.
        ratio: f64,
    },
    /// The fit would move the mesh farther than its own size — a mis-click,
    /// not a registration.
    Runaway {
        /// Translation length the fit produced, in millimetres.
        moved_by: f64,
        /// Largest translation considered plausible, in millimetres.
        allowed: f64,
    },
    /// A supplied point or normal was not finite.
    NonFinite,
}

/// Every axis undetermined — the report for a configuration that constrains
/// nothing at all.
const ALL_AXES_WEAK: FitRejection = FitRejection::Degenerate {
    weak_axes: [true, true, true],
};

/// Fit `moving` onto `fixed`.
///
/// `normals`, when supplied, are the clicked surface normals for each side and
/// are used only in the two-pair case. `moving_extent` is the moving mesh's
/// bounding-box diagonal in millimetres; a fit translating farther than that
/// is refused as [`FitRejection::Runaway`].
///
/// # Errors
///
/// Returns the [`FitRejection`] describing why no trustworthy pose exists.
pub fn fit_pairs(
    moving: &[DVec3],
    fixed: &[DVec3],
    normals: Option<(&[DVec3], &[DVec3])>,
    moving_extent: f64,
) -> Result<PairFit, FitRejection> {
    if moving.len() != fixed.len() {
        return Err(FitRejection::Unpaired {
            moving: moving.len(),
            fixed: fixed.len(),
        });
    }
    if moving.len() < MIN_PAIRS {
        return Err(FitRejection::TooFewPairs {
            have: moving.len(),
            need: MIN_PAIRS,
        });
    }
    if moving.iter().chain(fixed).any(|point| !point.is_finite()) {
        return Err(FitRejection::NonFinite);
    }

    let unit_ratio = distance_ratio(moving, fixed);
    if !(UNIT_RATIO_LOW..=UNIT_RATIO_HIGH).contains(&unit_ratio) {
        return Err(FitRejection::UnitMismatch { ratio: unit_ratio });
    }

    if moving.len() == MIN_PAIRS {
        let rigid = two_pair_frame(moving, fixed, normals)?;
        let residuals = residuals_of(&rigid, moving, fixed, &[0, 1]);
        return finish(rigid, &residuals, Vec::new(), unit_ratio, moving_extent);
    }

    let mut keep: Vec<usize> = (0..moving.len()).collect();
    let mut rejected: Vec<u32> = Vec::new();
    let mut rigid = horn_fit(moving, fixed, &keep)?;
    while keep.len() > 3 {
        let residuals = residuals_of(&rigid, moving, fixed, &keep);
        let Some(worst) = worst_outlier(&residuals) else {
            break;
        };
        let dropped = keep.remove(worst);
        rejected.push(u32::try_from(dropped).unwrap_or(u32::MAX));
        rigid = horn_fit(moving, fixed, &keep)?;
    }
    rejected.sort_unstable();
    let residuals = residuals_of(&rigid, moving, fixed, &keep);
    finish(rigid, &residuals, rejected, unit_ratio, moving_extent)
}

/// Apply the runaway guard and package the diagnostics.
fn finish(
    rigid: Rigid,
    residuals: &[f64],
    rejected: Vec<u32>,
    unit_ratio: f64,
    moving_extent: f64,
) -> Result<PairFit, FitRejection> {
    if !rigid.is_finite() {
        return Err(FitRejection::NonFinite);
    }
    let moved_by = rigid.translation.length();
    let allowed = moving_extent.max(1.0);
    if moved_by > allowed {
        return Err(FitRejection::Runaway { moved_by, allowed });
    }
    #[allow(clippy::cast_precision_loss)]
    let count = residuals.len().max(1) as f64;
    let sum_squares: f64 = residuals.iter().map(|value| value * value).sum();
    Ok(PairFit {
        rigid,
        pair_rms: (sum_squares / count).sqrt(),
        max_pair_err: residuals.iter().copied().fold(0.0, f64::max),
        rejected,
        unit_ratio,
    })
}

/// Horn's closed-form fit over the pairs named by `keep`.
///
/// The quaternion form is deliberate: it can only ever produce a proper
/// rotation, so a mirrored point set yields the best real rotation instead of
/// a reflection that would silently turn a scan inside out.
fn horn_fit(moving: &[DVec3], fixed: &[DVec3], keep: &[usize]) -> Result<Rigid, FitRejection> {
    let moving_centroid = centroid(moving, keep);
    let fixed_centroid = centroid(fixed, keep);
    if let Some(rejection) = line_degeneracy(moving, keep, moving_centroid) {
        return Err(rejection);
    }
    let mut covariance = DMat3::ZERO;
    for &index in keep {
        covariance += outer(
            moving[index] - moving_centroid,
            fixed[index] - fixed_centroid,
        );
    }
    let rotation = horn_quaternion(&covariance);
    Ok(Rigid::new(
        rotation,
        fixed_centroid - rotation * moving_centroid,
    ))
}

/// Whether the kept points lie on a line (or coincide), and which world axes
/// to name if they do.
///
/// This is a rank test on the spread itself, not a per-world-axis check: a
/// diagonal line has spread along every world axis and would sail through the
/// naive test while determining no rotation at all.
fn line_degeneracy(points: &[DVec3], keep: &[usize], centroid: DVec3) -> Option<FitRejection> {
    let mut spread = DMat3::ZERO;
    let mut total = 0.0f64;
    for &index in keep {
        let offset = points[index] - centroid;
        spread += outer(offset, offset);
        total += offset.length_squared();
    }
    if total <= f64::MIN_POSITIVE {
        return Some(ALL_AXES_WEAK);
    }
    let direction = dominant_direction(&spread);
    if direction.length_squared() <= 0.0 {
        return Some(ALL_AXES_WEAK);
    }
    let perpendicular: f64 = keep
        .iter()
        .map(|&index| {
            (points[index] - centroid)
                .reject_from_normalized(direction)
                .length_squared()
        })
        .sum();
    if perpendicular > total * COLLINEAR_FRACTION {
        return None;
    }
    let weak_axes = [
        direction.dot(DVec3::X).abs() >= WEAK_AXIS_SHARE,
        direction.dot(DVec3::Y).abs() >= WEAK_AXIS_SHARE,
        direction.dot(DVec3::Z).abs() >= WEAK_AXIS_SHARE,
    ];
    Some(FitRejection::Degenerate { weak_axes })
}

/// Unit direction of greatest spread, by power iteration from the strongest
/// diagonal — deterministic, and enough for a rank test.
fn dominant_direction(spread: &DMat3) -> DVec3 {
    let diagonal = [
        spread.x_axis.x.abs(),
        spread.y_axis.y.abs(),
        spread.z_axis.z.abs(),
    ];
    let mut best = 0usize;
    for index in 1..3 {
        if diagonal[index] > diagonal[best] {
            best = index;
        }
    }
    let mut vector = [DVec3::X, DVec3::Y, DVec3::Z][best];
    for _ in 0..POWER_ITERATIONS {
        let next = *spread * vector;
        if next.length_squared() <= f64::MIN_POSITIVE {
            return vector;
        }
        vector = next.normalize();
    }
    vector
}

/// The rotation carrying the moving points onto the fixed ones, as the
/// eigenvector of the largest eigenvalue of Horn's symmetric 4x4 matrix.
///
/// The `sxx`/`sxy`/... names are Horn's own published symbols. Renaming them
/// to satisfy a similarity lint would make the matrix below impossible to
/// check line-by-line against the paper, which is the only way anyone verifies
/// it.
#[allow(clippy::similar_names)]
fn horn_quaternion(covariance: &DMat3) -> DQuat {
    let entries = covariance.to_cols_array_2d();
    let (sxx, sxy, sxz) = (entries[0][0], entries[1][0], entries[2][0]);
    let (syx, syy, syz) = (entries[0][1], entries[1][1], entries[2][1]);
    let (szx, szy, szz) = (entries[0][2], entries[1][2], entries[2][2]);
    let matrix = [
        [sxx + syy + szz, syz - szy, szx - sxz, sxy - syx],
        [syz - szy, sxx - syy - szz, sxy + syx, szx + sxz],
        [szx - sxz, sxy + syx, -sxx + syy - szz, syz + szy],
        [sxy - syx, szx + sxz, syz + szy, -sxx - syy + szz],
    ];
    let [real, i, j, k] = dominant_eigenvector(matrix);
    let quaternion = DQuat::from_xyzw(i, j, k, real);
    if quaternion.length_squared() > 0.0 {
        quaternion.normalize()
    } else {
        DQuat::IDENTITY
    }
}

/// Eigenvector of the largest eigenvalue of a symmetric 4x4, by cyclic Jacobi.
///
/// The sweep is inherently index-paired — each rotation zeroes one specific
/// off-diagonal entry `(row, column)` and then updates both that row and that
/// column — so it is written with explicit indices rather than iterators.
#[allow(clippy::needless_range_loop)]
fn dominant_eigenvector(mut matrix: [[f64; 4]; 4]) -> [f64; 4] {
    let mut basis = [[0.0f64; 4]; 4];
    for (index, row) in basis.iter_mut().enumerate() {
        row[index] = 1.0;
    }
    for _ in 0..JACOBI_SWEEPS {
        let mut off_diagonal = 0.0;
        for row in 0..4 {
            for column in (row + 1)..4 {
                off_diagonal += matrix[row][column] * matrix[row][column];
            }
        }
        if off_diagonal <= f64::EPSILON {
            break;
        }
        for row in 0..4 {
            for column in (row + 1)..4 {
                if matrix[row][column].abs() <= f64::MIN_POSITIVE {
                    continue;
                }
                let theta =
                    (matrix[column][column] - matrix[row][row]) / (2.0 * matrix[row][column]);
                let sign = if theta >= 0.0 { 1.0 } else { -1.0 };
                let tangent = sign / (theta.abs() + theta.mul_add(theta, 1.0).sqrt());
                let cosine = 1.0 / tangent.mul_add(tangent, 1.0).sqrt();
                let sine = tangent * cosine;
                rotate_columns(&mut matrix, row, column, cosine, sine);
                for index in 0..4 {
                    let left = matrix[row][index];
                    let right = matrix[column][index];
                    matrix[row][index] = cosine * left - sine * right;
                    matrix[column][index] = sine * left + cosine * right;
                }
                rotate_columns(&mut basis, row, column, cosine, sine);
            }
        }
    }
    let mut best = 0usize;
    for index in 1..4 {
        if matrix[index][index] > matrix[best][best] {
            best = index;
        }
    }
    [
        basis[0][best],
        basis[1][best],
        basis[2][best],
        basis[3][best],
    ]
}

/// Apply one Jacobi rotation to a pair of columns.
fn rotate_columns(matrix: &mut [[f64; 4]; 4], left: usize, right: usize, cosine: f64, sine: f64) {
    for row in matrix {
        let first = row[left];
        let second = row[right];
        row[left] = cosine * first - sine * second;
        row[right] = sine * first + cosine * second;
    }
}

/// The two-pair fit: a right-handed frame per side from the segment and the
/// clicked surface normal.
fn two_pair_frame(
    moving: &[DVec3],
    fixed: &[DVec3],
    normals: Option<(&[DVec3], &[DVec3])>,
) -> Result<Rigid, FitRejection> {
    let Some((moving_normals, fixed_normals)) = normals else {
        return Err(ALL_AXES_WEAK);
    };
    if moving_normals.len() < MIN_PAIRS || fixed_normals.len() < MIN_PAIRS {
        return Err(ALL_AXES_WEAK);
    }
    if moving_normals
        .iter()
        .chain(fixed_normals)
        .any(|normal| !normal.is_finite())
    {
        return Err(FitRejection::NonFinite);
    }
    let moving_frame = frame_from(moving[0], moving[1], moving_normals[0])?;
    let fixed_frame = frame_from(fixed[0], fixed[1], fixed_normals[0])?;
    let rotation = DQuat::from_mat3(&(fixed_frame * moving_frame.transpose()));
    let moving_centroid = (moving[0] + moving[1]) * 0.5;
    let fixed_centroid = (fixed[0] + fixed[1]) * 0.5;
    let rigid = Rigid::new(rotation, fixed_centroid - rotation * moving_centroid);
    if rigid
        .apply_normal(moving_normals[1])
        .dot(fixed_normals[1].normalize_or_zero())
        < 0.0
    {
        return Err(ALL_AXES_WEAK);
    }
    Ok(rigid)
}

/// A right-handed basis from a segment and a surface normal at its start.
fn frame_from(start: DVec3, end: DVec3, normal: DVec3) -> Result<DMat3, FitRejection> {
    let along = (end - start).normalize_or_zero();
    let unit_normal = normal.normalize_or_zero();
    if along.length_squared() <= 0.0 || unit_normal.length_squared() <= 0.0 {
        return Err(ALL_AXES_WEAK);
    }
    if along.dot(unit_normal).abs() >= NORMAL_ALONG_SEGMENT_LIMIT {
        return Err(ALL_AXES_WEAK);
    }
    let up = unit_normal
        .reject_from_normalized(along)
        .normalize_or_zero();
    if up.length_squared() <= 0.0 {
        return Err(ALL_AXES_WEAK);
    }
    Ok(DMat3::from_cols(along, up, along.cross(up)))
}

/// Mean of the points named by `keep`.
fn centroid(points: &[DVec3], keep: &[usize]) -> DVec3 {
    if keep.is_empty() {
        return DVec3::ZERO;
    }
    let sum = keep
        .iter()
        .fold(DVec3::ZERO, |total, &index| total + points[index]);
    #[allow(clippy::cast_precision_loss)]
    let count = keep.len() as f64;
    sum / count
}

/// Outer product `left ⊗ right`, the matrix whose entry `(i, j)` is
/// `left[i] * right[j]`.
fn outer(left: DVec3, right: DVec3) -> DMat3 {
    DMat3::from_cols(left * right.x, left * right.y, left * right.z)
}

/// Residual distance per kept pair, in the order of `keep`.
fn residuals_of(rigid: &Rigid, moving: &[DVec3], fixed: &[DVec3], keep: &[usize]) -> Vec<f64> {
    keep.iter()
        .map(|&index| (rigid.apply(moving[index]) - fixed[index]).length())
        .collect()
}

/// Position within `residuals` of the pair worth dropping, if any.
fn worst_outlier(residuals: &[f64]) -> Option<usize> {
    let (position, value) = residuals.iter().enumerate().fold(
        None::<(usize, f64)>,
        |best, (index, &value)| match best {
            Some((_, current)) if current >= value => best,
            _ => Some((index, value)),
        },
    )?;
    let mut sorted = residuals.to_vec();
    sorted.sort_by(f64::total_cmp);
    let median = sorted.get(sorted.len() / 2).copied().unwrap_or(0.0);
    (value > TRIM_FLOOR_MM && value > median * TRIM_MEDIAN_FACTOR).then_some(position)
}

/// Mean fixed pairwise distance over mean moving pairwise distance.
fn distance_ratio(moving: &[DVec3], fixed: &[DVec3]) -> f64 {
    let mut moving_total = 0.0;
    let mut fixed_total = 0.0;
    for left in 0..moving.len() {
        for right in (left + 1)..moving.len() {
            moving_total += moving[left].distance(moving[right]);
            fixed_total += fixed[left].distance(fixed[right]);
        }
    }
    if moving_total <= f64::EPSILON {
        return 1.0;
    }
    fixed_total / moving_total
}
