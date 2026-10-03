//! Area moments and geometric proposal fits, independently derived from Horn
//! (1987), <https://doi.org/10.1364/JOSAA.4.000629>. Numerical span checks here
//! describe a proposal's solvability; the operator pair-fit confidence floor
//! does not apply. A proper quaternion never estimates scale or reflection.

use crate::{Rigid, SurfaceSample};
use glam::{DMat3, DQuat, DVec3};
use occluview_geometry::surface::{GeometryControl, GeometryStop, SurfaceIndex};

/// Serial facet-area moments avoid a density-dependent or spatial-split PCA
/// frame. The online centered covariance does not subtract large raw moments.
pub(crate) fn principal_index_frame(
    index: &SurfaceIndex,
    control: &GeometryControl,
) -> Result<Option<(DVec3, [DVec3; 3])>, GeometryStop> {
    let mut total = 0.;
    let mut center = DVec3::ZERO;
    let mut covariance = DMat3::ZERO;
    for (_, points, _) in index.triangles() {
        control.charge_operations(1)?;
        let weight = (points[1] - points[0])
            .cross(points[2] - points[0])
            .length()
            * 0.5;
        let point = points[0] / 3. + points[1] / 3. + points[2] / 3.;
        let next = total + weight;
        let delta = point - center;
        covariance += outer(delta, delta) * (weight * total / next);
        center += delta * (weight / next);
        total = next;
    }
    if total <= 0. {
        return Ok(None);
    }
    let Some((_, axes)) = symmetric_eigen3(covariance / total) else {
        return Err(GeometryStop::Numerical);
    };
    Ok(Some((
        center,
        [axes[2], axes[1], axes[2].cross(axes[1]).normalize_or_zero()],
    )))
}

/// Fit at most 4,096 finite geometric pairs with a 0.2 mm off-line span.
/// Missing or deficient correspondences return no hypothesis, never a panic.
pub(crate) fn fit_geometry_pairs(source: &[DVec3], target: &[DVec3]) -> Option<Rigid> {
    if source.len() != target.len() || !(3..=4_096).contains(&source.len()) {
        return None;
    }
    let moments = |points: &[DVec3]| {
        #[allow(clippy::cast_precision_loss)]
        let weight = 1. / points.len() as f64;
        let mut center = DVec3::ZERO;
        for &p in points {
            if !p.is_finite() {
                return None;
            }
            center += p * weight;
        }
        let mut covariance = DMat3::ZERO;
        for &p in points {
            covariance += outer(p - center, p - center) * weight;
        }
        let (values, _) = symmetric_eigen3(covariance)?;
        (values[1] >= 0.2f64.powi(2) && values[1] >= values[2] * 1e-6).then_some(center)
    };
    let source_center = moments(source)?;
    let target_center = moments(target)?;
    let mut covariance = DMat3::ZERO;
    for (&p, &q) in source.iter().zip(target) {
        covariance += outer(p - source_center, q - target_center);
    }
    if !covariance.is_finite() {
        return None;
    }
    let rotation =
        crate::rotation_grid::canonical_rotation(crate::pairs::horn_quaternion(&covariance));
    let pose = Rigid {
        rotation,
        translation: target_center - rotation * source_center,
    };
    pose.is_finite().then_some(pose)
}

/// Serial area-weighted principal frame; planar/repeated eigenvalues remain proposals.
pub(crate) fn principal_frame(
    samples: &[SurfaceSample],
    control: &GeometryControl,
) -> Result<Option<(DVec3, [DVec3; 3])>, GeometryStop> {
    let mut total = 0.;
    for sample in samples {
        control.charge_operations(1)?;
        total += sample.area_weight_mm2;
    }
    if !total.is_finite() || total <= 0. {
        return Ok(None);
    }
    let mut center = DVec3::ZERO;
    for sample in samples {
        control.charge_operations(1)?;
        center += sample.point * (sample.area_weight_mm2 / total);
    }
    let mut covariance = DMat3::ZERO;
    for sample in samples {
        control.charge_operations(1)?;
        covariance +=
            outer(sample.point - center, sample.point - center) * (sample.area_weight_mm2 / total);
    }
    let Some((_, axes)) = symmetric_eigen3(covariance) else {
        return Ok(None);
    };
    Ok(Some((
        center,
        [axes[2], axes[1], axes[2].cross(axes[1]).normalize_or_zero()],
    )))
}

fn outer(a: DVec3, b: DVec3) -> DMat3 {
    DMat3::from_cols(a * b.x, a * b.y, a * b.z)
}

/// Fixed largest-off-diagonal Jacobi pivots; ascending eigenvalues and paired vectors.
#[expect(
    clippy::needless_range_loop,
    clippy::many_single_char_names,
    reason = "fixed symmetric Jacobi pivot algebra"
)]
fn symmetric_eigen3(matrix: DMat3) -> Option<([f64; 3], [DVec3; 3])> {
    if !matrix.is_finite() {
        return None;
    }
    let mut a = matrix.to_cols_array_2d();
    let mut v = [[0.; 3]; 3];
    for i in 0..3 {
        v[i][i] = 1.;
    }
    for _ in 0..48 {
        let (mut p, mut q) = (0, 1);
        for (i, j) in [(0, 2), (1, 2)] {
            if a[i][j].abs() > a[p][q].abs() {
                p = i;
                q = j;
            }
        }
        if a[p][q].abs() <= 1e-14 {
            break;
        }
        let angle = 0.5 * (2. * a[p][q]).atan2(a[q][q] - a[p][p]);
        let (s, c) = angle.sin_cos();
        let pp = c * c * a[p][p] - 2. * c * s * a[p][q] + s * s * a[q][q];
        let qq = s * s * a[p][p] + 2. * c * s * a[p][q] + c * c * a[q][q];
        for i in 0..3 {
            if i != p && i != q {
                let first = c * a[i][p] - s * a[i][q];
                let second = s * a[i][p] + c * a[i][q];
                a[i][p] = first;
                a[p][i] = first;
                a[i][q] = second;
                a[q][i] = second;
            }
            let first = c * v[i][p] - s * v[i][q];
            let second = s * v[i][p] + c * v[i][q];
            v[i][p] = first;
            v[i][q] = second;
        }
        a[p][p] = pp;
        a[q][q] = qq;
        a[p][q] = 0.;
        a[q][p] = 0.;
    }
    let mut order = [0, 1, 2];
    order.sort_by(|&i, &j| a[i][i].total_cmp(&a[j][j]).then(i.cmp(&j)));
    Some((
        order.map(|i| a[i][i]),
        order.map(|i| DVec3::new(v[0][i], v[1][i], v[2][i])),
    ))
}

/// All 24 right-handed matches between two orthonormal frames.
pub(crate) fn proper_frame_matches(moving: [DVec3; 3], fixed: [DVec3; 3]) -> Vec<DQuat> {
    let mut result = Vec::with_capacity(24);
    let source = DMat3::from_cols(moving[0], moving[1], moving[2]);
    for permutation in [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ] {
        for bits in 0..8 {
            let sign = |i| if bits & (1 << i) == 0 { 1. } else { -1. };
            let target = DMat3::from_cols(
                fixed[permutation[0]] * sign(0),
                fixed[permutation[1]] * sign(1),
                fixed[permutation[2]] * sign(2),
            );
            if target.determinant() > 0. {
                let matrix = target * source.transpose();
                if matrix.is_finite() && (matrix.determinant() - 1.).abs() < 1e-8 {
                    result.push(crate::rotation_grid::canonical_rotation(DQuat::from_mat3(
                        &matrix,
                    )));
                }
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geometric_proposals_fit_proper_motion_and_decline_invalid_span() {
        let points = [DVec3::ZERO, DVec3::X * 4., DVec3::Y * 3., DVec3::Z * 2.];
        let truth = Rigid::new(
            DQuat::from_axis_angle(DVec3::new(2., 3., 7.).normalize(), 2.137),
            DVec3::new(1e6, -1e6, 1e6),
        );
        let target = points.map(|p| truth.apply(p));
        let fit = fit_geometry_pairs(&points, &target).unwrap();
        assert!(points
            .into_iter()
            .all(|p| fit.apply(p).distance(truth.apply(p)) < 1e-8));
        assert!((DMat3::from_quat(fit.rotation).determinant() - 1.).abs() < 1e-10);
        for bad in [
            [DVec3::ZERO; 4],
            [DVec3::ZERO, DVec3::X, DVec3::X * 2., DVec3::X * 3.],
            [DVec3::splat(f64::NAN); 4],
            [
                DVec3::splat(1e300),
                DVec3::splat(-1e300),
                DVec3::X * 1e300,
                DVec3::Y * 1e300,
            ],
        ] {
            assert!(fit_geometry_pairs(&bad, &target).is_none());
        }
        assert!(fit_geometry_pairs(&[], &[]).is_none());
        assert!(fit_geometry_pairs(&points[..3], &target).is_none());
    }
}
