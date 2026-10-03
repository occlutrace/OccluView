//! Finite rival probes derived from the geometric null-space analysis in
//! Gelfand et al. (2003),
//! <https://pixl.cs.princeton.edu/pubs/Gelfand_2003_GSS/stabicp.pdf>.
//! Proper principal half-turns and weak twists test alternate geometric basins.
//! This bounded set is evidence, never a certificate of global uniqueness.

use crate::{Metric, MissingReason, PreparedSurface, Rigid};
use glam::DVec3;
use occluview_geometry::surface::{GeometryControl, GeometryStop};

/// Three principal half-turns and both signs of .5/2 mm normalized weak motion,
/// at most 27 finite corrections. Returned poses are in the private query frame.
/// Tooth-pitch autocorrelation is not covered by this set and must remain an
/// explicit unfinished mandatory pass at the publication boundary.
///
/// # Errors
/// Returns the actual control or arithmetic stop; no partial set is certified.
pub(crate) fn probe_rivals(
    moving: &PreparedSurface,
    pose: Rigid,
    weak: &[[f64; 6]],
    control: &GeometryControl,
) -> Result<Vec<Rigid>, GeometryStop> {
    let mut probes = Vec::new();
    probes
        .try_reserve_exact(32)
        .map_err(|_| GeometryStop::ResourceLimit)?;
    let Some((local_center, axes)) =
        crate::proposal_geometry::principal_frame(&moving.samples[3].samples, control)?
    else {
        return Ok(probes);
    };
    let center = pose.apply(local_center);
    for axis in axes {
        control.charge_operations(1)?;
        let trial = super::super::icp_solve::apply_step(
            pose,
            center,
            (pose.rotation * axis) * std::f64::consts::PI,
            DVec3::ZERO,
        );
        if trial.is_finite() {
            probes.push(trial);
        }
    }
    let mut total = 0.;
    let mut radius2 = 0.;
    for sample in &moving.samples[3].samples {
        control.charge_operations(1)?;
        total += sample.area_weight_mm2;
        radius2 += sample.area_weight_mm2 * sample.point.distance_squared(local_center);
    }
    let radius = (radius2 / total).sqrt();
    if !radius.is_finite() || radius <= 0.5 {
        return Ok(probes);
    }
    for twist in weak.iter().take(6) {
        if twist.iter().any(|v| !v.is_finite()) {
            return Err(GeometryStop::Numerical);
        }
        for displacement in [-2., -0.5, 0.5, 2.] {
            control.charge_operations(1)?;
            let rotation = DVec3::new(twist[0], twist[1], twist[2]) * (displacement / radius);
            let translation = DVec3::new(twist[3], twist[4], twist[5]) * displacement;
            let trial = super::super::icp_solve::apply_step(pose, center, rotation, translation);
            if trial.is_finite() {
                probes.push(trial);
            }
        }
    }
    Ok(probes)
}

/// Relative gap from fully evaluated distinct rivals. An absent set is missing,
/// and a better rival gives zero gap. Missing support is never a zero score.
pub(crate) fn rival_gap(
    best: Metric<f64>,
    rivals: impl IntoIterator<Item = Metric<f64>>,
) -> Metric<f64> {
    let Metric::Measured(best) = best else {
        return Metric::Missing(MissingReason::NotEvaluated);
    };
    if !best.is_finite() || best < 0. {
        return Metric::Missing(MissingReason::NotEvaluated);
    }
    let rival = rivals
        .into_iter()
        .filter_map(|m| match m {
            Metric::Measured(v) if v.is_finite() && v >= 0. => Some(v),
            _ => None,
        })
        .max_by(f64::total_cmp);
    rival.map_or(Metric::Missing(MissingReason::NoSupport), |r| {
        Metric::Measured(((best - r) / best.max(0.01)).clamp(0., 1.))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn absent_rivals_do_not_manufacture_a_perfect_gap() {
        assert_eq!(
            rival_gap(Metric::Measured(0.8), []),
            Metric::Missing(MissingReason::NoSupport)
        );
        assert_eq!(
            rival_gap(
                Metric::Measured(0.8),
                [Metric::Missing(MissingReason::Interrupted)]
            ),
            Metric::Missing(MissingReason::NoSupport)
        );
        assert_eq!(
            rival_gap(Metric::Measured(0.8), [Metric::Measured(0.)]),
            Metric::Measured(1.)
        );
        assert_eq!(
            rival_gap(Metric::Measured(0.8), [Metric::Measured(0.9)]),
            Metric::Measured(0.)
        );
    }
}
