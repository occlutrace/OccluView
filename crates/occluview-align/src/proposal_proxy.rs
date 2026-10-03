//! Bounded proposal-only surface representatives. Triangle selection uses
//! area strata (Osada et al., 2002,
//! <https://gfx.cs.princeton.edu/pubs/Osada_2002_SD/tog02.pdf>), independently
//! combined with the existing controlled triangle index. Omitted facets can
//! only worsen this surrogate distance. Every retained pose is rescored on
//! the complete original surface; proxy residuals are never final evidence.

use crate::{MeshInput, PreparedSurface, SurfaceIndex};
use occluview_geometry::surface::{BuildOutcome, GeometryControl, GeometryStop, Soup};

/// Construct at most 1,024 area-selected original facets without baking any
/// positions. Source ids address validated original topology; the same f64
/// authored-to-query affine conditions both representations.
pub(crate) fn proposal_proxy(
    input: MeshInput<'_>,
    surface: &PreparedSurface,
    control: &GeometryControl,
) -> Result<Option<SurfaceIndex>, GeometryStop> {
    if !surface.exact_original {
        return Ok(None);
    }
    let samples = &surface.samples[1].samples;
    let count = samples.len().min(1_024);
    let _memory = control.reserve(count * 16 + 64)?;
    let mut ids = Vec::new();
    ids.try_reserve_exact(count)
        .map_err(|_| GeometryStop::ResourceLimit)?;
    for i in 0..count {
        control.charge_operations(1)?;
        ids.push(samples[i * samples.len() / count].triangle);
    }
    ids.sort_unstable();
    ids.dedup();
    let mut indices = Vec::new();
    indices
        .try_reserve_exact(ids.len() * 3)
        .map_err(|_| GeometryStop::ResourceLimit)?;
    for id in ids {
        control.charge_operations(1)?;
        let start = (id as usize)
            .checked_mul(3)
            .ok_or(GeometryStop::Numerical)?;
        let triangle = input
            .soup
            .indices
            .get(start..start + 3)
            .ok_or(GeometryStop::Numerical)?;
        indices.extend_from_slice(triangle);
    }
    match SurfaceIndex::build_controlled(
        Soup {
            positions: input.soup.positions,
            indices: &indices,
            mask: input.soup.mask,
        },
        surface.frame.query_from_local,
        control,
    ) {
        BuildOutcome::Complete(index) => Ok(Some(index)),
        BuildOutcome::Empty => Ok(None),
        BuildOutcome::Partial { reason, .. } => Err(reason),
    }
}
