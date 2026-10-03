//! Area-weighted robust rigid updates, independently derived from Gelfand et
//! al. (2003), <https://pixl.cs.princeton.edu/pubs/Gelfand_2003_GSS/stabicp.pdf>.
//! A centred, radius-normalized plane Jacobian exposes unconstrained twists.
//! Huber IRLS uses a bounded centred MAD scale. Damping never creates evidence
//! or moves a null mode. Unsigned vector residuals supply a numerical fallback
//! without manufacturing plane information.

use crate::{RefinementTermination, Rigid};
use glam::DVec3;
use occluview_geometry::surface::{GeometryControl, GeometryStop};

#[derive(Clone, Copy)]
pub(super) struct SurfacePair {
    pub moving: DVec3,
    pub fixed: DVec3,
    pub normal: Option<DVec3>,
    pub weight: f64,
}

#[derive(Clone, Debug)]
pub(crate) struct PlaneInformation {
    pub values: [f64; 6],
    pub weak: Vec<[f64; 6]>,
}

pub(super) struct Linearization {
    pub matrix: [[f64; 6]; 6],
    pub gradient: [f64; 6],
    pub center: DVec3,
    pub radius: f64,
    pub delta: f64,
    pub fallback: bool,
    pub information: Option<PlaneInformation>,
}

/// Weighted quantile in stable value order. Sorting work is charged even if
/// the caller cancels during the bounded sort; interrupted output is discarded.
pub(super) fn quantile(
    values: &mut [(f64, f64)],
    fraction: f64,
    control: &GeometryControl,
) -> Result<f64, GeometryStop> {
    let mut stopped = None;
    values.sort_by(|a, b| {
        if stopped.is_none() {
            stopped = control.charge_operations(1).err();
        }
        a.0.total_cmp(&b.0)
    });
    if let Some(stop) = stopped {
        return Err(stop);
    }
    let total: f64 = values.iter().map(|v| v.1).sum();
    let mut accumulated = 0.;
    for &(value, weight) in values.iter() {
        control.charge_operations(1)?;
        accumulated += weight;
        if accumulated >= total * fraction {
            return Ok(value);
        }
    }
    Ok(0.)
}

pub(super) fn accumulate_robust(
    pairs: &[SurfacePair],
    pose: Rigid,
    control: &GeometryControl,
) -> Result<Option<Linearization>, GeometryStop> {
    if pairs.len() < 6 {
        return Ok(None);
    }
    let fallback = pairs.iter().any(|p| p.normal.is_none());
    let mut center = DVec3::ZERO;
    let mut total = 0.;
    let mut residuals = Vec::new();
    residuals
        .try_reserve_exact(pairs.len())
        .map_err(|_| GeometryStop::ResourceLimit)?;
    for p in pairs {
        control.charge_operations(1)?;
        let point = pose.apply(p.moving);
        let difference = point - p.fixed;
        let r = if fallback {
            difference.length()
        } else {
            difference.dot(p.normal.unwrap_or(DVec3::ZERO))
        };
        if !point.is_finite() || !r.is_finite() || !p.weight.is_finite() || p.weight <= 0. {
            return Ok(None);
        }
        let next = total + p.weight;
        center += (point - center) * (p.weight / next);
        total = next;
        residuals.push((r, p.weight));
    }
    let median = quantile(&mut residuals, 0.5, control)?;
    for r in &mut residuals {
        control.charge_operations(1)?;
        r.0 = (r.0 - median).abs();
    }
    let delta = 1.345 * (1.4826 * quantile(&mut residuals, 0.5, control)?).clamp(0.02, 0.20);
    let mut squared_radius = 0.;
    for p in pairs {
        control.charge_operations(1)?;
        squared_radius += pose.apply(p.moving).distance_squared(center) * (p.weight / total);
    }
    let radius = squared_radius.sqrt();
    if !center.is_finite() || !radius.is_finite() || radius <= 1e-12 || !total.is_finite() {
        return Ok(None);
    }
    let mut matrix = [[0.; 6]; 6];
    let mut gradient = [0.; 6];
    let mut plane = [[0.; 6]; 6];
    for p in pairs {
        control.charge_operations(1)?;
        let point = pose.apply(p.moving);
        let d = point - p.fixed;
        let r = if fallback {
            d.length()
        } else {
            d.dot(p.normal.unwrap_or(DVec3::ZERO))
        };
        let robust = (delta / r.abs().max(delta)).min(1.);
        let weight = p.weight / total;
        if fallback {
            for axis in [DVec3::X, DVec3::Y, DVec3::Z] {
                let moment = (point - center).cross(axis) / radius;
                let j = [moment.x, moment.y, moment.z, axis.x, axis.y, axis.z];
                add_row(&mut matrix, &mut gradient, j, d.dot(axis), weight * robust);
            }
        } else {
            let normal = p.normal.unwrap_or(DVec3::ZERO);
            let moment = (point - center).cross(normal) / radius;
            let j = [moment.x, moment.y, moment.z, normal.x, normal.y, normal.z];
            add_row(&mut matrix, &mut gradient, j, r, weight * robust);
            let mut unused = [0.; 6];
            add_row(&mut plane, &mut unused, j, 0., weight);
        }
    }
    if matrix
        .iter()
        .flatten()
        .chain(gradient.iter())
        .any(|v| !v.is_finite())
    {
        return Ok(None);
    }
    let information = (!fallback).then(|| information(&plane));
    Ok(Some(Linearization {
        matrix,
        gradient,
        center,
        radius,
        delta,
        fallback,
        information,
    }))
}

fn add_row(matrix: &mut [[f64; 6]; 6], gradient: &mut [f64; 6], j: [f64; 6], r: f64, w: f64) {
    for (i, row) in matrix.iter_mut().enumerate() {
        gradient[i] -= w * j[i] * r;
        for (k, value) in row.iter_mut().enumerate() {
            *value += w * j[i] * j[k];
        }
    }
}

fn information(matrix: &[[f64; 6]; 6]) -> PlaneInformation {
    let (values, vectors) = super::icp_solve::symmetric_eigendecomposition(*matrix);
    let mut order = [0, 1, 2, 3, 4, 5];
    order.sort_by(|&a, &b| values[a].total_cmp(&values[b]));
    let maximum = values.into_iter().fold(0., f64::max);
    PlaneInformation {
        values: order.map(|i| values[i]),
        weak: order
            .into_iter()
            .filter(|&i| values[i] <= maximum * 1e-6)
            .map(|i| std::array::from_fn(|row| vectors[row][i]))
            .collect(),
    }
}

/// Project the gradient onto observable modes of the undamped matrix. The
/// normalized angular components are divided by patch radius by the caller.
pub(super) fn solve_observable_step(
    matrix: &[[f64; 6]; 6],
    gradient: [f64; 6],
) -> Option<[f64; 6]> {
    if matrix
        .iter()
        .flatten()
        .chain(gradient.iter())
        .any(|v| !v.is_finite())
    {
        return None;
    }
    let (values, vectors) = super::icp_solve::symmetric_eigendecomposition(*matrix);
    let largest = values.into_iter().fold(0., f64::max);
    if largest <= 0. || !largest.is_finite() {
        return None;
    }
    let mut damping = 1e-6;
    for _ in 0..4 {
        let mut result = [0.; 6];
        for (mode, &value) in values.iter().enumerate() {
            if value / largest <= 1e-6 {
                continue;
            }
            let projection: f64 = gradient
                .iter()
                .zip(&vectors)
                .map(|(g, row)| g * row[mode])
                .sum();
            let coefficient = projection / (value + damping * largest);
            for (axis, value) in result.iter_mut().enumerate() {
                *value += coefficient * vectors[axis][mode];
            }
        }
        if result.iter().all(|v| v.is_finite()) {
            return Some(result);
        }
        damping *= 10.;
    }
    None
}

pub(super) fn frozen_objective(
    pairs: &[SurfacePair],
    pose: Rigid,
    model: &Linearization,
    control: &GeometryControl,
) -> Result<f64, GeometryStop> {
    let mut objective = 0.;
    let mut weight = 0.;
    for p in pairs {
        control.charge_operations(1)?;
        let d = pose.apply(p.moving) - p.fixed;
        let r = if model.fallback {
            d.length()
        } else {
            d.dot(p.normal.unwrap_or(DVec3::ZERO)).abs()
        };
        let cost = if r <= model.delta {
            0.5 * r * r
        } else {
            model.delta * (r - 0.5 * model.delta)
        };
        objective += cost * p.weight;
        weight += p.weight;
    }
    Ok(objective / weight.max(f64::MIN_POSITIVE))
}

/// Frozen correspondences, prefix, area weights and robust scale at every
/// backtrack. No seating veto, seed tether or distance-from-start refusal.
/// An accepted step carries its completed before/after objective reductions;
/// callers can evaluate stability without repeating identical serial sums.
pub(super) fn line_search(
    pairs: &[SurfacePair],
    pose: Rigid,
    model: &Linearization,
    dense: bool,
    control: &GeometryControl,
) -> Result<(Rigid, RefinementTermination, Option<[f64; 2]>), GeometryStop> {
    let Some(step) = solve_observable_step(&model.matrix, model.gradient) else {
        return Ok((pose, RefinementTermination::Singular, None));
    };
    let mut rotation = DVec3::new(step[0], step[1], step[2]) / model.radius;
    let mut translation = DVec3::new(step[3], step[4], step[5]);
    let angular_bound = if dense {
        1f64.to_radians()
    } else {
        5f64.to_radians()
    };
    let translation_bound = if dense { 0.1 } else { 0.5 };
    if !rotation.is_finite() || !translation.is_finite() {
        return Ok((pose, RefinementTermination::NumericalTrialRejected, None));
    }
    rotation = capped_vector(rotation, angular_bound);
    translation = capped_vector(translation, translation_bound);
    if rotation.length() <= 1e-5 && translation.length() <= 1e-4 {
        return Ok((pose, RefinementTermination::StepSmall, None));
    }
    let before = frozen_objective(pairs, pose, model, control)?;
    for fraction in [1., 0.5, 0.25, 0.125] {
        let trial = super::icp_solve::apply_step(
            pose,
            model.center,
            rotation * fraction,
            translation * fraction,
        );
        if !trial.is_finite() {
            continue;
        }
        let after = frozen_objective(pairs, trial, model, control)?;
        if after.is_finite() && after <= before * (1. - 1e-4) {
            return Ok((
                trial,
                RefinementTermination::NotStarted,
                Some([before, after]),
            ));
        }
    }
    Ok((pose, RefinementTermination::Stationary, None))
}

fn capped_vector(vector: DVec3, bound: f64) -> DVec3 {
    let maximum = vector.abs().max_element();
    if maximum == 0. {
        return vector;
    }
    let scaled = vector / maximum;
    let scale = bound / scaled.length();
    if maximum > scale {
        scaled * scale
    } else {
        vector
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepted_line_search_returns_the_exact_completed_objectives() {
        let mut pairs = analytic_pairs(0);
        for pair in &mut pairs {
            pair.fixed.z = 0.1;
        }
        let unlimited = GeometryControl::unlimited();
        let model = accumulate_robust(&pairs, Rigid::IDENTITY, &unlimited)
            .unwrap()
            .unwrap();
        let control = GeometryControl::new(
            crate::CancelFlag::new(),
            std::time::Duration::MAX,
            occluview_geometry::surface::GeometryLimits {
                operations: pairs.len() as u64 * 2,
                ..occluview_geometry::surface::GeometryLimits::default()
            },
        );
        let (next, termination, objectives) =
            line_search(&pairs, Rigid::IDENTITY, &model, false, &control).unwrap();
        assert_eq!(termination, RefinementTermination::NotStarted);
        assert_eq!(
            objectives,
            Some([
                frozen_objective(&pairs, Rigid::IDENTITY, &model, &unlimited).unwrap(),
                frozen_objective(&pairs, next, &model, &unlimited).unwrap(),
            ])
        );
        assert_eq!(control.counters().operations, pairs.len() as u64 * 2);
        assert_eq!(control.counters().query_calls, 0);
        let cancel = crate::CancelFlag::new();
        cancel.cancel();
        let cancelled = GeometryControl::new(
            cancel,
            std::time::Duration::MAX,
            occluview_geometry::surface::GeometryLimits::default(),
        );
        assert!(matches!(
            line_search(&pairs, Rigid::IDENTITY, &model, false, &cancelled),
            Err(GeometryStop::Cancelled)
        ));
    }

    #[test]
    fn overflowing_twist_is_rejected_without_losing_the_pose() {
        let mut model = Linearization {
            matrix: [[0.; 6]; 6],
            gradient: [0.; 6],
            center: DVec3::ZERO,
            radius: 1e-11,
            delta: 0.0269,
            fallback: false,
            information: None,
        };
        for (i, row) in model.matrix.iter_mut().enumerate() {
            row[i] = 1.;
        }
        model.gradient[0] = 1e308;
        let result = line_search(
            &analytic_pairs(0),
            Rigid::IDENTITY,
            &model,
            false,
            &GeometryControl::unlimited(),
        )
        .unwrap();
        assert_eq!(result.0, Rigid::IDENTITY);
        assert_eq!(result.1, RefinementTermination::NumericalTrialRejected);
        assert_eq!(capped_vector(DVec3::X * 1e308, 0.5), DVec3::X * 0.5);
    }

    fn analytic_pairs(kind: u8) -> Vec<SurfacePair> {
        let mut pairs = Vec::new();
        for i in 0..256u32 {
            let angle = std::f64::consts::TAU * (f64::from(i) + 0.5) / 256.;
            let z = -0.9 + 1.8 * (f64::from(i * 73 % 256) + 0.5) / 256.;
            let (point, normal) = match kind {
                0 => (
                    DVec3::new(10. * angle.cos(), 10. * angle.sin(), 0.),
                    DVec3::Z,
                ),
                1 => (
                    DVec3::new(10. * angle.cos(), 10. * angle.sin(), 10. * z),
                    DVec3::new(angle.cos(), angle.sin(), 0.),
                ),
                _ => {
                    let n = DVec3::new(
                        (1. - z * z).sqrt() * angle.cos(),
                        (1. - z * z).sqrt() * angle.sin(),
                        z,
                    );
                    (n * 10., n)
                }
            };
            pairs.push(SurfacePair {
                moving: point,
                fixed: point,
                normal: Some(normal),
                weight: 1.,
            });
            if kind == 2 {
                pairs.push(SurfacePair {
                    moving: -point,
                    fixed: -point,
                    normal: Some(-normal),
                    weight: 1.,
                });
            }
        }
        pairs
    }
    /// ID25 analytic: a plane retains two sliding directions and its free roll.
    #[test]
    fn same_plane_has_free_motion_information() {
        let model = accumulate_robust(
            &analytic_pairs(0),
            Rigid::IDENTITY,
            &GeometryControl::unlimited(),
        )
        .unwrap()
        .unwrap();
        let information = model.information.unwrap();
        assert!(information.values[0] <= 1e-8);
        assert!(information.weak.len() >= 3);
        let step = solve_observable_step(&model.matrix, [1.; 6]).unwrap();
        assert!(step[2].abs() < 1e-12 && step[3].abs() < 1e-12 && step[4].abs() < 1e-12);
    }
    /// ID26 analytic: cylinder axial translation and axial roll are null.
    #[test]
    fn same_cylinder_has_free_motion_information() {
        let model = accumulate_robust(
            &analytic_pairs(1),
            Rigid::IDENTITY,
            &GeometryControl::unlimited(),
        )
        .unwrap()
        .unwrap();
        let information = model.information.unwrap();
        assert!(information.values[0] <= 1e-8);
        assert!(information.weak.len() >= 2);
        let step = solve_observable_step(&model.matrix, [1.; 6]).unwrap();
        assert!(step[2].abs() < 1e-12 && step[5].abs() < 1e-12);
    }
    /// ID27 analytic: spherical radial normals constrain no rotation.
    #[test]
    fn same_sphere_has_free_rotations_information() {
        let model = accumulate_robust(
            &analytic_pairs(2),
            Rigid::IDENTITY,
            &GeometryControl::unlimited(),
        )
        .unwrap()
        .unwrap();
        let information = model.information.unwrap();
        assert!(information.values[0] <= 1e-8);
        assert!(information.weak.len() >= 3);
        let step = solve_observable_step(&model.matrix, [1.; 6]).unwrap();
        assert!(step[..3].iter().all(|v| v.abs() < 1e-12));
    }
    #[test]
    fn missing_normals_never_manufacture_plane_information() {
        let mut pairs = analytic_pairs(2);
        pairs[0].normal = None;
        let model = accumulate_robust(&pairs, Rigid::IDENTITY, &GeometryControl::unlimited())
            .unwrap()
            .unwrap();
        assert!(model.fallback && model.information.is_none());
    }
    #[test]
    fn plane_normal_sign_and_robust_scale_are_consistent() {
        let mut pairs = analytic_pairs(0);
        for (i, p) in pairs.iter_mut().enumerate() {
            p.fixed.z = if i < 25 { 10. } else { 0.1 };
        }
        let first = accumulate_robust(&pairs, Rigid::IDENTITY, &GeometryControl::unlimited())
            .unwrap()
            .unwrap();
        for p in &mut pairs {
            p.normal = p.normal.map(|n| -n);
        }
        let second = accumulate_robust(&pairs, Rigid::IDENTITY, &GeometryControl::unlimited())
            .unwrap()
            .unwrap();
        assert_eq!(first.matrix, second.matrix);
        assert_eq!(first.gradient, second.gradient);
        assert!((first.delta - 1.345 * 0.02).abs() < 1e-12);
        let (pose, _, _) = line_search(
            &pairs,
            Rigid::IDENTITY,
            &second,
            true,
            &GeometryControl::unlimited(),
        )
        .unwrap();
        assert!(pose.translation.z > 0.09 && pose.translation.z <= 0.1);
        assert!(pose.translation.x.abs() < 1e-12 && pose.translation.y.abs() < 1e-12);
    }
    #[test]
    fn invalid_equations_and_cancelled_reductions_keep_no_step() {
        assert!(solve_observable_step(&[[f64::NAN; 6]; 6], [0.; 6]).is_none());
        assert!(solve_observable_step(&[[0.; 6]; 6], [f64::INFINITY; 6]).is_none());
        assert!(
            accumulate_robust(&[], Rigid::IDENTITY, &GeometryControl::unlimited())
                .unwrap()
                .is_none()
        );
        let flag = crate::CancelFlag::new();
        flag.cancel();
        let control = GeometryControl::new(
            flag,
            std::time::Duration::MAX,
            occluview_geometry::surface::GeometryLimits::default(),
        );
        assert!(matches!(
            accumulate_robust(&analytic_pairs(0), Rigid::IDENTITY, &control),
            Err(GeometryStop::Cancelled)
        ));
    }
}
