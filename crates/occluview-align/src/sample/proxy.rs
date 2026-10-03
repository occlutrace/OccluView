//! Bounded vertex clustering, independently derived from Rossignac and Borrel
//! (1993), <https://doi.org/10.1007/978-3-642-78114-8_3>. One source vertex per
//! occupied cell preserves physical coordinates and winding. Source triangles
//! are streamed in order; collapsed/duplicate facets are omitted. This surrogate
//! is explicitly approximate and supplies no original-surface certificate.

use super::vertex_at;
use crate::{MeshInput, RegionPolicy, SurfaceFrame, SurfaceIndex};
use occluview_geometry::surface::{BuildOutcome, GeometryControl, GeometryStop, Soup};
use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasherDefault, DefaultHasher};

const VERTICES: usize = 32_768;
const TRIANGLES: usize = 64_000;

/// Full-source deterministic attempts; a capacity stop retries a coarser cell,
/// never an arbitrary triangle prefix. Coordinates remain source f32 widened
/// through the same authored f64 affine at index construction.
#[expect(
    clippy::too_many_lines,
    reason = "one bounded representation transaction with full-source retries"
)]
pub(super) fn prepare_bounded_proxy(
    mesh: MeshInput<'_>,
    frame: SurfaceFrame,
    regions: RegionPolicy,
    control: &GeometryControl,
) -> Result<SurfaceIndex, GeometryStop> {
    let _memory = control.reserve(VERTICES * 160 + TRIANGLES * 96 + 4096)?;
    for attempt in 0..8 {
        let cell = 0.02 * 2f64.powi(attempt);
        let mut vertices = HashMap::<_, _, BuildHasherDefault<DefaultHasher>>::default();
        let mut faces = HashSet::<_, BuildHasherDefault<DefaultHasher>>::default();
        let mut positions = Vec::new();
        let mut indices = Vec::new();
        vertices
            .try_reserve(VERTICES)
            .map_err(|_| GeometryStop::ResourceLimit)?;
        faces
            .try_reserve(TRIANGLES)
            .map_err(|_| GeometryStop::ResourceLimit)?;
        positions
            .try_reserve_exact(VERTICES * 3)
            .map_err(|_| GeometryStop::ResourceLimit)?;
        indices
            .try_reserve_exact(TRIANGLES * 3)
            .map_err(|_| GeometryStop::ResourceLimit)?;
        let mut full = true;
        for triangle in mesh.soup.indices.as_chunks::<3>().0 {
            control.charge_operations(1)?;
            if crate::mask::eligible_region(mesh.soup, *triangle, regions).is_none() {
                continue;
            }
            let mut mapped = [0; 3];
            let mut valid = true;
            for (corner, &id) in triangle.iter().enumerate() {
                control.charge_operations(1)?;
                let Some(point) = vertex_at(mesh.soup.positions, id as usize) else {
                    valid = false;
                    break;
                };
                let query = frame.query_from_local.transform_point3(point);
                let scaled = (query / cell).floor();
                if !scaled.is_finite() {
                    return Err(GeometryStop::Numerical);
                }
                // Float-bit cell keys avoid saturating integer casts on huge inputs.
                let key = scaled.to_array().map(|v| (v + 0.).to_bits());
                let index = if let Some(&index) = vertices.get(&key) {
                    index
                } else {
                    if vertices.len() >= VERTICES {
                        full = false;
                        break;
                    }
                    let index =
                        u32::try_from(vertices.len()).map_err(|_| GeometryStop::ResourceLimit)?;
                    vertices.insert(key, index);
                    let offset = id as usize * 3;
                    let Some(position) = mesh.soup.positions.get(offset..offset + 3) else {
                        valid = false;
                        break;
                    };
                    positions.extend_from_slice(position);
                    index
                };
                mapped[corner] = index;
            }
            if !full {
                break;
            }
            if !valid || mapped[0] == mapped[1] || mapped[1] == mapped[2] || mapped[0] == mapped[2]
            {
                continue;
            }
            let first = mapped
                .iter()
                .enumerate()
                .min_by_key(|(_, id)| **id)
                .map_or(0, |p| p.0);
            let canonical = [
                mapped[first],
                mapped[(first + 1) % 3],
                mapped[(first + 2) % 3],
            ];
            if faces.contains(&canonical) {
                continue;
            }
            if faces.len() >= TRIANGLES {
                full = false;
                break;
            }
            faces.insert(canonical);
            indices.extend_from_slice(&mapped);
        }
        if !full {
            continue;
        }
        drop(vertices);
        drop(faces);
        return match SurfaceIndex::build_controlled(
            Soup {
                positions: &positions,
                indices: &indices,
                mask: None,
            },
            frame.query_from_local,
            control,
        ) {
            BuildOutcome::Complete(index) => Ok(index),
            BuildOutcome::Empty => Err(GeometryStop::ResourceLimit),
            BuildOutcome::Partial { reason, .. } => Err(reason),
        };
    }
    Err(GeometryStop::ResourceLimit)
}
