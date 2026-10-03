//! Exact area populations in disjoint spatial cells, independently derived
//! from Sutherland and Hodgman (1974), <https://doi.org/10.1145/360767.360802>.
//! Six half-space clips partition each eligible triangle into 1 mm cubes.
//! Area-stratified barycentric points follow Osada et al. (2002),
//! <https://gfx.cs.princeton.edu/pubs/Osada_2002_SD/tog02.pdf>.
//! Cell roles have measured area; no half-population extrapolation is used.

use super::prepared::{cell_hash, completion, empty_batch, uniform};
use crate::{SampleBatch, SampleRole, SurfaceIndex, SurfaceSample};
use glam::DVec3;
use occluview_geometry::surface::{GeometryControl, GeometryMemory, GeometryStop};

#[derive(Clone, Copy, Default)]
struct Vertex {
    point: DVec3,
    barycentric: DVec3,
}
struct Facet {
    vertices: [Vertex; 3],
    normal: Option<DVec3>,
    triangle: u32,
    end: f64,
}
pub(super) struct CellPopulation {
    facets: [Vec<Facet>; 2],
    areas: [f64; 2],
    _memory: [Option<GeometryMemory>; 2],
}

impl CellPopulation {
    /// Fully partition eligible triangles, or explicitly stop without a prefix
    /// that could claim complete population area. Scratch is fallibly admitted.
    #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
    pub(super) fn build(
        index: &SurfaceIndex,
        normals: bool,
        control: &GeometryControl,
    ) -> Result<Self, GeometryStop> {
        // A broad triangle can span many cells even in a tiny mesh. Cap
        // fragments directly, rather than assuming a per-facet expansion.
        let cap = 262_144;
        let mut memory = [None, None];
        let mut facets = [Vec::new(), Vec::new()];
        let mut areas = [0.; 2];
        for (triangle, points, normal) in index.triangles() {
            control.charge_operations(1)?;
            let lower = points[0].min(points[1]).min(points[2]).floor();
            let upper = points[0].max(points[1]).max(points[2]).floor();
            if !lower.is_finite()
                || !upper.is_finite()
                || lower.abs().max_element().max(upper.abs().max_element()) >= 1e9
            {
                return Err(GeometryStop::Numerical);
            }
            let lo = lower.to_array().map(|x| x as i64);
            let hi = upper.to_array().map(|x| x as i64);
            let cells = (0..3).try_fold(1u64, |n, i| {
                n.checked_mul(u64::try_from(hi[i] - lo[i] + 1).ok()?)
            });
            if cells.is_none_or(|n| n > 131_072) {
                return Err(GeometryStop::ResourceLimit);
            }
            for x in lo[0]..=hi[0] {
                for y in lo[1]..=hi[1] {
                    for z in lo[2]..=hi[2] {
                        control.charge_operations(1)?;
                        let cell = [x, y, z];
                        let mut polygon = [Vertex::default(); 12];
                        for i in 0..3 {
                            polygon[i] = Vertex {
                                point: points[i],
                                barycentric: [DVec3::X, DVec3::Y, DVec3::Z][i],
                            };
                        }
                        let mut len = 3;
                        for (axis, coordinate) in cell.into_iter().enumerate() {
                            for (plane, positive) in
                                [(coordinate as f64, true), (coordinate as f64 + 1., false)]
                            {
                                len = clip(&mut polygon, len, axis, plane, positive, control)?;
                            }
                        }
                        if len < 3 {
                            continue;
                        }
                        let role = usize::from(
                            cell_hash(DVec3::new(x as f64 + 0.5, y as f64 + 0.5, z as f64 + 0.5))?
                                & 8
                                != 0,
                        );
                        for i in 1..len - 1 {
                            control.charge_operations(1)?;
                            let v = [polygon[0], polygon[i], polygon[i + 1]];
                            let area = (v[1].point - v[0].point)
                                .cross(v[2].point - v[0].point)
                                .length()
                                * 0.5;
                            if area <= 0. {
                                continue;
                            }
                            areas[role] += area;
                            if !areas[role].is_finite() {
                                return Err(GeometryStop::Numerical);
                            }
                            if facets[role].len() >= cap {
                                return Err(GeometryStop::ResourceLimit);
                            }
                            if facets[role].len() == facets[role].capacity() {
                                let capacity =
                                    facets[role].capacity().saturating_add(2048).min(cap);
                                // Admit both old and new buffers during possible relocation.
                                let admitted = control.reserve(capacity * size_of::<Facet>())?;
                                facets[role]
                                    .try_reserve_exact(capacity - facets[role].len())
                                    .map_err(|_| GeometryStop::ResourceLimit)?;
                                memory[role] = Some(admitted);
                            }
                            facets[role].push(Facet {
                                vertices: v,
                                normal: normals.then_some(normal),
                                triangle,
                                end: areas[role],
                            });
                        }
                    }
                }
            }
        }
        Ok(Self {
            facets,
            areas,
            _memory: memory,
        })
    }

    #[allow(clippy::cast_precision_loss)]
    pub(super) fn samples(
        &self,
        role: SampleRole,
        budget: usize,
        seed: u64,
        control: &GeometryControl,
    ) -> Result<SampleBatch, GeometryStop> {
        let which = usize::from(role == SampleRole::Holdout);
        let area = self.areas[which];
        let mut batch = empty_batch(budget, control)?;
        batch.role = role;
        batch.population_area_mm2 = area;
        // The other role is outside this declared population. Its contribution
        // to whole-surface support remains the explicit [0, omitted area] bound.
        batch.unqueried_area_mm2 = self.areas[1 - which];
        if budget == 0 || area <= 0. {
            return Ok(batch);
        }
        let mut state = seed;
        let mut slot = 0;
        let weight = area / budget as f64;
        for id in 0..budget {
            if let Err(stop) = control.charge_operations(1) {
                batch.completion = completion(stop);
                break;
            }
            let coordinate = (id as f64 + uniform(&mut state)) * weight;
            while slot + 1 < self.facets[which].len() && self.facets[which][slot].end <= coordinate
            {
                control.charge_operations(1)?;
                slot += 1;
            }
            let facet = self.facets[which]
                .get(slot)
                .ok_or(GeometryStop::Numerical)?;
            let radius = uniform(&mut state).sqrt();
            let angular = uniform(&mut state);
            let weights = [1. - radius, radius * (1. - angular), radius * angular];
            let mut point = DVec3::ZERO;
            let mut barycentric = DVec3::ZERO;
            for (vertex, w) in facet.vertices.iter().zip(weights) {
                point += vertex.point * w;
                barycentric += vertex.barycentric * w;
            }
            batch.samples.push(SurfaceSample {
                id: u32::try_from(id).map_err(|_| GeometryStop::ResourceLimit)?,
                point,
                normal: facet.normal,
                triangle: facet.triangle,
                barycentric: barycentric.to_array(),
                area_weight_mm2: weight,
                region_id: 0,
            });
        }
        batch.represented_area_mm2 = weight * batch.samples.len() as f64;
        Ok(batch)
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "fixed polygon buffer and clipping half-space"
)]
fn clip(
    polygon: &mut [Vertex; 12],
    len: usize,
    axis: usize,
    plane: f64,
    positive: bool,
    control: &GeometryControl,
) -> Result<usize, GeometryStop> {
    if len == 0 {
        return Ok(0);
    }
    let mut output = [Vertex::default(); 12];
    let mut count = 0;
    let inside = |v: Vertex| {
        if positive {
            v.point[axis] >= plane
        } else {
            v.point[axis] <= plane
        }
    };
    let mut previous = polygon[len - 1];
    for &current in &polygon[..len] {
        control.charge_operations(1)?;
        if inside(previous) != inside(current) {
            let t = (plane - previous.point[axis]) / (current.point[axis] - previous.point[axis]);
            let mut point = previous.point.lerp(current.point, t);
            point[axis] = plane;
            let v = Vertex {
                point,
                barycentric: previous.barycentric.lerp(current.barycentric, t),
            };
            *output.get_mut(count).ok_or(GeometryStop::ResourceLimit)? = v;
            count += 1;
        }
        if inside(current) {
            *output.get_mut(count).ok_or(GeometryStop::ResourceLimit)? = current;
            count += 1;
        }
        previous = current;
    }
    *polygon = output;
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Soup;

    #[test]
    fn clipped_cell_area_matches_independent_rectangle_integral() {
        let positions = [
            0.25, 0.25, 0., 2.25, 0.25, 0., 2.25, 2.25, 0., 0.25, 2.25, 0.,
        ];
        let index = SurfaceIndex::build(Soup {
            positions: &positions,
            indices: &[0, 1, 2, 0, 2, 3],
            mask: None,
        })
        .unwrap();
        let control = GeometryControl::unlimited();
        let population = CellPopulation::build(&index, true, &control).unwrap();
        let mut exact = [0.; 2];
        for (x, width) in [0.75, 1., 0.25].into_iter().enumerate() {
            for (y, height) in [0.75, 1., 0.25].into_iter().enumerate() {
                let center = DVec3::new(
                    f64::from(u32::try_from(x).unwrap()) + 0.5,
                    f64::from(u32::try_from(y).unwrap()) + 0.5,
                    0.5,
                );
                exact[usize::from(cell_hash(center).unwrap() & 8 != 0)] += width * height;
            }
        }
        for (area, expected) in population.areas.iter().zip(exact) {
            assert!((*area - expected).abs() <= 1e-12);
        }
        assert!((population.areas.iter().sum::<f64>() - 4.).abs() <= 1e-12);
        let training = population
            .samples(SampleRole::Training, 1024, 1, &control)
            .unwrap();
        let holdout = population
            .samples(SampleRole::Holdout, 8192, 2, &control)
            .unwrap();
        for batch in [&training, &holdout] {
            let total: f64 = batch.samples.iter().map(|s| s.area_weight_mm2).sum();
            assert!((total / batch.population_area_mm2 - 1.).abs() <= 1e-12);
            for sample in &batch.samples {
                assert_eq!(
                    cell_hash(sample.point).unwrap() & 8 != 0,
                    batch.role == SampleRole::Holdout
                );
                assert!(sample
                    .barycentric
                    .iter()
                    .all(|&w| (0. - 1e-12..=1. + 1e-12).contains(&w)));
                assert!((sample.barycentric.iter().sum::<f64>() - 1.).abs() <= 1e-12);
            }
        }
        drop(training);
        drop(holdout);
        drop(population);
        assert_eq!(control.counters().memory_bytes, 0);
    }
}
