//! Principal axes (PCA) of a point cloud: the eigenvectors of the covariance
//! matrix of vertex positions, sorted by descending variance, plus the
//! centroid they were computed about.
//!
//! This is the STABLE, per-mesh-constant "global shape" signal the cut disc
//! (Bridge Split and Cut View both drive [`crate::cut_manipulator`] logic
//! through it) anchors its orientation to, instead of the hit triangle's
//! local normal: a dental arch or bridge span's own axes and centroid never
//! jitter as the cursor crosses triangles, and the LOCAL direction from the
//! centroid to a point on the surface, projected onto the `axes[0]`/`axes[1]`
//! plane, rotates smoothly around the arch — reducing to (roughly) `axes[0]`
//! at the arch's left/right extremes, the anatomically useful orientation for
//! viewing occlusal contacts there, and adapting continuously in between
//! instead of staying fixed for the whole mesh.

use glam::Vec3;

/// A point cloud's PCA centroid and its three orthonormal axes, sorted by
/// DESCENDING variance: `axes[0]` is the direction of greatest spread,
/// `axes[2]` the least. Right-handed (`axes[2] == axes[0].cross(axes[1])`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PrincipalFrame {
    /// Mean position of the points the frame was computed from.
    pub centroid: Vec3,
    /// `[greatest, middle, least]`-variance orthonormal axes.
    pub axes: [Vec3; 3],
}

/// The [`PrincipalFrame`] of `points`.
///
/// `None` when fewer than 3 finite points are present, or the point cloud has
/// no well-defined axis (every point coincident).
#[must_use]
pub fn principal_frame(points: impl Iterator<Item = Vec3> + Clone) -> Option<PrincipalFrame> {
    let mut count: u64 = 0;
    let mut centroid = Vec3::ZERO;
    for p in points.clone() {
        if !p.is_finite() {
            continue;
        }
        centroid += p;
        count += 1;
    }
    if count < 3 {
        return None;
    }
    #[allow(clippy::cast_precision_loss)]
    let centroid = centroid / count as f32;

    // Covariance matrix in f64 (symmetric; only the upper triangle is
    // accumulated) to avoid the catastrophic cancellation a large, far-from-
    // origin point cloud would otherwise hit in f32.
    let mut cov = [[0.0_f64; 3]; 3];
    let mut finite_count: u64 = 0;
    for p in points {
        if !p.is_finite() {
            continue;
        }
        let d = (p - centroid).as_dvec3();
        cov[0][0] += d.x * d.x;
        cov[0][1] += d.x * d.y;
        cov[0][2] += d.x * d.z;
        cov[1][1] += d.y * d.y;
        cov[1][2] += d.y * d.z;
        cov[2][2] += d.z * d.z;
        finite_count += 1;
    }
    if finite_count < 3 {
        return None;
    }
    cov[1][0] = cov[0][1];
    cov[2][0] = cov[0][2];
    cov[2][1] = cov[1][2];

    let (eigenvalues, eigenvectors) = jacobi_eigen_symmetric_3x3(cov)?;

    // Every point coincident: the covariance matrix is all zeros, so Jacobi
    // hands back the identity basis as "eigenvectors" with zero eigenvalues —
    // a valid orthonormal frame with no actual relationship to the (nonexistent)
    // spread of the data. Refuse it rather than returning an arbitrary axis.
    let largest_eigenvalue = eigenvalues.iter().copied().fold(0.0_f64, f64::max);
    if largest_eigenvalue <= 1e-12 {
        return None;
    }

    let mut order = [0usize, 1, 2];
    order.sort_by(|&a, &b| {
        eigenvalues[b]
            .partial_cmp(&eigenvalues[a])
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // f64 -> f32: eigenvector components of a normalized-ish direction, well
    // within f32 range; only precision (not magnitude) is lost.
    #[allow(clippy::cast_possible_truncation)]
    let axis = |i: usize| -> Vec3 {
        Vec3::new(
            eigenvectors[0][i] as f32,
            eigenvectors[1][i] as f32,
            eigenvectors[2][i] as f32,
        )
    };
    let a0 = axis(order[0]).normalize_or_zero();
    let a1_raw = axis(order[1]).normalize_or_zero();
    if a0.length_squared() <= f32::EPSILON || a1_raw.length_squared() <= f32::EPSILON {
        return None;
    }
    // Jacobi already gives orthogonal eigenvectors for a symmetric matrix;
    // re-orthogonalize defensively against floating-point drift and derive
    // the third axis via cross product for an exactly orthonormal,
    // guaranteed-right-handed frame (never a reflection).
    let a1 = (a1_raw - a0 * a0.dot(a1_raw)).normalize_or_zero();
    if a1.length_squared() <= f32::EPSILON {
        return None;
    }
    let a2 = a0.cross(a1).normalize_or_zero();
    if a2.length_squared() <= f32::EPSILON {
        return None;
    }
    Some(PrincipalFrame {
        centroid,
        axes: [a0, a1, a2],
    })
}

/// Hard sweep bound for [`jacobi_eigen_symmetric_3x3`] (a safety valve; the
/// off-diagonal tolerance below converges well before this in practice).
const JACOBI_MAX_SWEEPS: usize = 64;
/// [`jacobi_eigen_symmetric_3x3`] convergence: stop once the sum of
/// off-diagonal magnitudes is negligible.
const JACOBI_CONVERGED_OFF_DIAGONAL: f64 = 1e-12;
/// [`jacobi_eigen_symmetric_3x3`]: skip a rotation for an already-negligible
/// pair (avoids a division by a near-zero pivot).
const JACOBI_NEGLIGIBLE_PAIR: f64 = 1e-15;

/// Cyclic Jacobi eigenvalue algorithm for a symmetric 3x3 matrix (Golub & Van
/// Loan's classical rotation method): repeatedly zero the largest off-
/// diagonal pair until the matrix is diagonal to tolerance. Returns
/// `(eigenvalues, eigenvectors)` where `eigenvectors[row][col]` is the
/// `col`-th eigenvector's `row`-th component. `None` if any input entry is
/// non-finite.
///
/// Single-letter names below (`a`, `v`, `p`, `q`, `c`, `s`, `t`) mirror the
/// standard Jacobi-rotation reference notation deliberately, so the
/// implementation stays checkable line-by-line against it.
#[allow(clippy::many_single_char_names)]
fn jacobi_eigen_symmetric_3x3(mut a: [[f64; 3]; 3]) -> Option<([f64; 3], [[f64; 3]; 3])> {
    for row in &a {
        for &entry in row {
            if !entry.is_finite() {
                return None;
            }
        }
    }
    let mut v = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

    for _ in 0..JACOBI_MAX_SWEEPS {
        let off = a[0][1].abs() + a[0][2].abs() + a[1][2].abs();
        if off <= JACOBI_CONVERGED_OFF_DIAGONAL {
            break;
        }
        for (p, q) in [(0usize, 1usize), (0, 2), (1, 2)] {
            if a[p][q].abs() <= JACOBI_NEGLIGIBLE_PAIR {
                continue;
            }
            let theta = (a[q][q] - a[p][p]) / (2.0 * a[p][q]);
            let t = if theta == 0.0 {
                1.0
            } else {
                theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt())
            };
            let c = 1.0 / (t * t + 1.0).sqrt();
            let s = t * c;
            let diag_p = a[p][p];
            let diag_q = a[q][q];
            let off_diag = a[p][q];
            a[p][p] = diag_p - t * off_diag;
            a[q][q] = diag_q + t * off_diag;
            a[p][q] = 0.0;
            a[q][p] = 0.0;
            for i in 0..3 {
                if i != p && i != q {
                    let row_p = a[i][p];
                    let row_q = a[i][q];
                    a[i][p] = c * row_p - s * row_q;
                    a[p][i] = a[i][p];
                    a[i][q] = s * row_p + c * row_q;
                    a[q][i] = a[i][q];
                }
            }
            for row in &mut v {
                let col_p = row[p];
                let col_q = row[q];
                row[p] = c * col_p - s * col_q;
                row[q] = s * col_p + c * col_q;
            }
        }
    }
    Some(([a[0][0], a[1][1], a[2][2]], v))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_unit_and_orthogonal(axes: [Vec3; 3]) {
        for axis in axes {
            assert!(
                (axis.length() - 1.0).abs() < 1e-4,
                "axis not unit length: {axis} ({})",
                axis.length()
            );
        }
        assert!(axes[0].dot(axes[1]).abs() < 1e-4);
        assert!(axes[0].dot(axes[2]).abs() < 1e-4);
        assert!(axes[1].dot(axes[2]).abs() < 1e-4);
        // Right-handed: axes[2] must equal axes[0] x axes[1], not its negation.
        let cross = axes[0].cross(axes[1]);
        assert!(
            cross.distance(axes[2]) < 1e-3,
            "frame is not right-handed: cross={cross} axes[2]={}",
            axes[2]
        );
    }

    #[test]
    fn elongated_cluster_reports_its_long_axis_first() {
        // Points scattered mostly along X (a stand-in for a bridge span / arch
        // long axis), with much smaller spread on Y and Z.
        let points: Vec<Vec3> = (0..40)
            .map(|i| {
                #[allow(clippy::cast_precision_loss)]
                let t = i as f32;
                Vec3::new(
                    t * 2.0 - 40.0,
                    (t * 0.7).sin() * 0.3,
                    (t * 1.3).cos() * 0.15,
                )
            })
            .collect();
        let axes = principal_frame(points.into_iter())
            .expect("well-defined axes")
            .axes;
        assert_unit_and_orthogonal(axes);
        assert!(
            axes[0].x.abs() > 0.99,
            "long axis should align with X: {}",
            axes[0]
        );
    }

    #[test]
    fn arch_like_curved_point_cloud_still_yields_a_stable_span_axis() {
        // A shallow U-shaped arc in the XZ plane (Y ~ vertical, tiny jitter) --
        // closer to a real dental arch than a straight line. The long axis
        // should still land close to the arc's overall left-right spread (X),
        // not its (comparatively tiny) height.
        let points: Vec<Vec3> = (0..60)
            .map(|i| {
                #[allow(clippy::cast_precision_loss)]
                let t = i as f32 / 59.0; // 0..1
                let angle = (t - 0.5) * std::f32::consts::PI * 0.6;
                Vec3::new(
                    angle.sin() * 30.0,
                    (t * 11.0).sin() * 0.4,
                    angle.cos() * 8.0,
                )
            })
            .collect();
        let axes = principal_frame(points.into_iter())
            .expect("well-defined axes")
            .axes;
        assert_unit_and_orthogonal(axes);
        assert!(
            axes[0].y.abs() < 0.2,
            "arch span axis should not tip into the vertical: {}",
            axes[0]
        );
    }

    #[test]
    fn two_points_return_none() {
        assert!(principal_frame([Vec3::ZERO, Vec3::X].into_iter()).is_none());
    }

    #[test]
    fn every_point_coincident_returns_none() {
        let points = vec![Vec3::new(5.0, 5.0, 5.0); 10];
        assert!(principal_frame(points.into_iter()).is_none());
    }

    #[test]
    fn non_finite_points_are_skipped_not_propagated() {
        let mut points: Vec<Vec3> = (0..20)
            .map(|i| {
                #[allow(clippy::cast_precision_loss)]
                Vec3::new(i as f32, 0.0, 0.0)
            })
            .collect();
        points.push(Vec3::new(f32::NAN, f32::INFINITY, 0.0));
        let axes = principal_frame(points.into_iter())
            .expect("finite points still resolve axes")
            .axes;
        assert_unit_and_orthogonal(axes);
        assert!(axes[0].x.abs() > 0.99);
    }

    #[test]
    fn perfectly_symmetric_cube_corners_still_produce_a_valid_orthonormal_frame() {
        // No unique long axis (all three extents equal), but the function
        // must still return SOME valid orthonormal right-handed frame rather
        // than degenerating.
        let points = vec![
            Vec3::new(-1.0, -1.0, -1.0),
            Vec3::new(1.0, -1.0, -1.0),
            Vec3::new(-1.0, 1.0, -1.0),
            Vec3::new(1.0, 1.0, -1.0),
            Vec3::new(-1.0, -1.0, 1.0),
            Vec3::new(1.0, -1.0, 1.0),
            Vec3::new(-1.0, 1.0, 1.0),
            Vec3::new(1.0, 1.0, 1.0),
        ];
        let axes = principal_frame(points.into_iter())
            .expect("cube corners are non-degenerate")
            .axes;
        assert_unit_and_orthogonal(axes);
    }

    #[test]
    fn is_deterministic_across_repeated_calls() {
        let points: Vec<Vec3> = (0..25)
            .map(|i| {
                #[allow(clippy::cast_precision_loss)]
                let t = i as f32;
                Vec3::new(t * 1.7, (t * 0.9).sin(), (t * 0.4).cos() * 2.0)
            })
            .collect();
        let a = principal_frame(points.iter().copied()).expect("frame");
        let b = principal_frame(points.iter().copied()).expect("frame");
        assert_eq!(a, b);
    }

    #[test]
    fn diagonal_covariance_recovers_axis_aligned_eigenvectors() {
        // A point cloud whose covariance is EXACTLY diagonal (axis-aligned
        // spread): symmetric about the origin along each axis independently,
        // with distinctly different variances so the ranking is unambiguous.
        let mut points = Vec::new();
        for &x in &[-3.0_f32, 3.0] {
            for &y in &[-2.0_f32, 2.0] {
                for &z in &[-1.0_f32, 1.0] {
                    points.push(Vec3::new(x, y, z));
                }
            }
        }
        let axes = principal_frame(points.into_iter())
            .expect("axis-aligned axes")
            .axes;
        assert_unit_and_orthogonal(axes);
        assert!(axes[0].x.abs() > 0.99, "expected X first: {}", axes[0]);
        assert!(axes[1].y.abs() > 0.99, "expected Y second: {}", axes[1]);
        assert!(axes[2].z.abs() > 0.99, "expected Z third: {}", axes[2]);
    }

    #[test]
    fn centroid_is_the_true_mean_of_the_points() {
        let points = [
            Vec3::new(2.0, 4.0, 6.0),
            Vec3::new(4.0, 8.0, 10.0),
            Vec3::new(6.0, 0.0, 2.0),
        ];
        let frame = principal_frame(points.into_iter()).expect("well-defined frame");
        assert!(
            frame.centroid.distance(Vec3::new(4.0, 4.0, 6.0)) < 1e-4,
            "expected the mean of the three points: {}",
            frame.centroid
        );
    }

    #[test]
    fn centroid_skips_non_finite_points_like_the_axes_do() {
        let mut points: Vec<Vec3> = (0..20)
            .map(|i| {
                #[allow(clippy::cast_precision_loss)]
                Vec3::new(i as f32, 0.0, 0.0)
            })
            .collect();
        points.push(Vec3::new(f32::NAN, f32::INFINITY, 0.0));
        let frame = principal_frame(points.into_iter()).expect("finite points still resolve");
        // Mean of 0..=19 is 9.5; the non-finite point must not have polluted it.
        assert!(
            (frame.centroid.x - 9.5).abs() < 1e-4,
            "non-finite point should not skew the centroid: {}",
            frame.centroid
        );
        assert_eq!(frame.centroid.y, 0.0);
    }
}
