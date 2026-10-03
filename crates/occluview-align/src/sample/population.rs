//! Exact area populations in disjoint spatial cells, independently derived
//! from Sutherland and Hodgman (1974), <https://doi.org/10.1145/360767.360802>.
//! Six half-space clips partition each eligible triangle into 1 mm cubes.
//! Area-stratified barycentric points follow Osada et al. (2002),
//! <https://gfx.cs.princeton.edu/pubs/Osada_2002_SD/tog02.pdf>.
//! Cell roles have measured area; no half-population extrapolation is used.

use super::prepared::{cell_hash, empty_batch, uniform};
use crate::{SampleBatch, SampleRole, SurfaceIndex, SurfaceSample};
use glam::DVec3;
use occluview_geometry::surface::{GeometryControl, GeometryStop};

#[derive(Clone, Copy, Default)]
struct Vertex {
    point: DVec3,
    barycentric: DVec3,
}
struct CellFacet {
    vertices: [Vertex; 3],
    normal: Option<DVec3>,
    triangle: u32,
    area: f64,
}
struct Request {
    coordinate: f64,
    stream: usize,
    id: u32,
    radius: f64,
    angular: f64,
}

/// Fixed storage and exact work for one synthetic cell-clipping diagnostic.
#[cfg(feature = "search-probe")]
#[derive(Clone, Debug, PartialEq)]
pub struct CellClipProbe {
    /// Active polygon vertices occupy the prefix ending at `len`.
    pub points: [DVec3; 12],
    /// Original-triangle coordinates of the corresponding vertices.
    pub barycentric: [DVec3; 12],
    /// Number of active vertices, at most twelve.
    pub len: usize,
    /// Ordinary units charged by clipping, excluding input validation.
    pub operations: u64,
}

/// Inspect the authoritative clipping of a synthetic triangle into one slab.
///
/// The slab includes both boundaries `[coordinate, coordinate + 1]` on `axis`.
/// Points and cell coordinates must have magnitude below 1e9. Degenerate
/// triangles are accepted; their polygon need not have positive area. Fixed
/// storage bounds work and allocation; serial control use isolates the count.
///
/// # Errors
/// Returns `Numerical` for non-finite points or an invalid axis, `ResourceLimit`
/// for unsupported coordinates, or the control's interruption reason.
#[cfg(feature = "search-probe")]
pub fn probe_cell_clipping(
    points: [DVec3; 3],
    axis: usize,
    coordinate: i64,
    control: &GeometryControl,
) -> Result<CellClipProbe, GeometryStop> {
    control.charge_operations(3)?;
    if axis >= 3 || points.iter().any(|p| !p.is_finite()) {
        return Err(GeometryStop::Numerical);
    }
    if coordinate.unsigned_abs() >= 1_000_000_000
        || points.iter().any(|p| p.abs().max_element() >= 1e9)
    {
        return Err(GeometryStop::ResourceLimit);
    }
    let mut polygon = [Vertex::default(); 12];
    for (i, point) in points.into_iter().enumerate() {
        polygon[i] = Vertex {
            point,
            barycentric: [DVec3::X, DVec3::Y, DVec3::Z][i],
        };
    }
    let before = control.counters().operations;
    let len = clip_cell(&mut polygon, 3, axis, coordinate, control)?;
    Ok(CellClipProbe {
        points: polygon.map(|v| v.point),
        barycentric: polygon.map(|v| v.barycentric),
        len,
        operations: control.counters().operations - before,
    })
}

/// Two serial passes retain area totals and requested samples, never all of the
/// potentially millions of clipped fragments. The second pass reconstructs
/// the same source-order CDF; every stream keeps its own fixed PRNG sequence.
pub(super) struct CellPopulation<'a> {
    index: &'a SurfaceIndex,
    normals: bool,
    areas: [f64; 2],
}

impl<'a> CellPopulation<'a> {
    pub(super) fn build(
        index: &'a SurfaceIndex,
        normals: bool,
        control: &GeometryControl,
    ) -> Result<Self, GeometryStop> {
        let mut areas = [0.; 2];
        visit_facets(index, normals, control, |role, facet| {
            areas[role] += facet.area;
            if !areas[role].is_finite() {
                return Err(GeometryStop::Numerical);
            }
            Ok(())
        })?;
        Ok(Self {
            index,
            normals,
            areas,
        })
    }

    /// Resolve up to four independently seeded streams in one reconstruction.
    /// Their sorted area coordinates select exact clipped role populations;
    /// no inclusion multiplier or facet-prefix truncation is introduced.
    #[expect(
        clippy::too_many_lines,
        reason = "bounded transactional streams over one immutable CDF"
    )]
    #[allow(clippy::cast_precision_loss)]
    pub(super) fn batches(
        &self,
        streams: &[(SampleRole, usize, u64)],
        control: &GeometryControl,
    ) -> Result<Vec<SampleBatch>, GeometryStop> {
        if streams.len() > 4 {
            return Err(GeometryStop::ResourceLimit);
        }
        let count = streams
            .iter()
            .try_fold(0usize, |n, (_, budget, _)| n.checked_add(*budget))
            .ok_or(GeometryStop::ResourceLimit)?;
        let _memory = control.reserve(
            count
                .checked_mul(size_of::<Request>() * 2)
                .and_then(|n| n.checked_add(256))
                .ok_or(GeometryStop::ResourceLimit)?,
        )?;
        let mut requests = [Vec::new(), Vec::new()];
        let mut batches = Vec::new();
        batches
            .try_reserve_exact(streams.len())
            .map_err(|_| GeometryStop::ResourceLimit)?;
        for (stream, &(role, budget, seed)) in streams.iter().enumerate() {
            if role == SampleRole::Full {
                return Err(GeometryStop::Numerical);
            }
            let which = usize::from(role == SampleRole::Holdout);
            let area = self.areas[which];
            let mut batch = empty_batch(budget, control)?;
            batch.role = role;
            batch.population_area_mm2 = area;
            batch.unqueried_area_mm2 = self.areas[1 - which];
            if budget != 0 && area > 0. {
                requests[which]
                    .try_reserve_exact(budget)
                    .map_err(|_| GeometryStop::ResourceLimit)?;
                let weight = area / budget as f64;
                let mut state = seed;
                for id in 0..budget {
                    control.charge_operations(1)?;
                    requests[which].push(Request {
                        coordinate: (id as f64 + uniform(&mut state)) * weight,
                        stream,
                        id: u32::try_from(id).map_err(|_| GeometryStop::ResourceLimit)?,
                        radius: uniform(&mut state).sqrt(),
                        angular: uniform(&mut state),
                    });
                }
            }
            batches.push(batch);
        }
        for role in &mut requests {
            let mut stopped = None;
            role.sort_by(|a, b| {
                if stopped.is_none() {
                    stopped = control.charge_operations(1).err();
                }
                a.coordinate
                    .total_cmp(&b.coordinate)
                    .then(a.stream.cmp(&b.stream))
                    .then(a.id.cmp(&b.id))
            });
            if let Some(stop) = stopped {
                return Err(stop);
            }
        }
        let mut accumulated = [0.; 2];
        let mut slots = [0; 2];
        visit_facets(self.index, self.normals, control, |role, facet| {
            accumulated[role] += facet.area;
            while let Some(request) = requests[role].get(slots[role]) {
                if request.coordinate >= accumulated[role] {
                    break;
                }
                control.charge_operations(1)?;
                let weights = [
                    1. - request.radius,
                    request.radius * (1. - request.angular),
                    request.radius * request.angular,
                ];
                let mut point = DVec3::ZERO;
                let mut barycentric = DVec3::ZERO;
                for (vertex, weight) in facet.vertices.iter().zip(weights) {
                    point += vertex.point * weight;
                    barycentric += vertex.barycentric * weight;
                }
                let batch = &mut batches[request.stream];
                batch.samples.push(SurfaceSample {
                    id: request.id,
                    point,
                    normal: facet.normal,
                    triangle: facet.triangle,
                    barycentric: barycentric.to_array(),
                    area_weight_mm2: self.areas[role] / streams[request.stream].1 as f64,
                    region_id: 0,
                });
                slots[role] += 1;
            }
            Ok(())
        })?;
        if slots
            .iter()
            .zip(&requests)
            .any(|(slot, requests)| *slot != requests.len())
        {
            return Err(GeometryStop::Numerical);
        }
        for (batch, (_, budget, _)) in batches.iter_mut().zip(streams) {
            if *budget > 0 {
                batch.represented_area_mm2 =
                    batch.population_area_mm2 / *budget as f64 * batch.samples.len() as f64;
            }
        }
        Ok(batches)
    }

    #[cfg(test)]
    pub(super) fn samples(
        &self,
        role: SampleRole,
        budget: usize,
        seed: u64,
        control: &GeometryControl,
    ) -> Result<SampleBatch, GeometryStop> {
        self.batches(&[(role, budget, seed)], control)?
            .pop()
            .ok_or(GeometryStop::Numerical)
    }
}

/// Clip in x/y/z order, sharing each x slice and x/y strip across its cells.
/// Strip bounds omit only cells with empty intersections; the resulting
/// positive-area fragments and serial summation order match the full lattice.
#[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
fn visit_facets(
    index: &SurfaceIndex,
    normals: bool,
    control: &GeometryControl,
    mut visit: impl FnMut(usize, CellFacet) -> Result<(), GeometryStop>,
) -> Result<(), GeometryStop> {
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
        let mut original = [Vertex::default(); 12];
        for i in 0..3 {
            original[i] = Vertex {
                point: points[i],
                barycentric: [DVec3::X, DVec3::Y, DVec3::Z][i],
            };
        }
        for x in lo[0]..=hi[0] {
            control.charge_operations(1)?;
            let mut x_polygon = original;
            let x_len = clip_cell(&mut x_polygon, 3, 0, x, control)?;
            if x_len < 3 {
                continue;
            }
            let (y_lo, y_hi) = axis_bounds(&x_polygon[..x_len], 1);
            for y in y_lo..=y_hi {
                control.charge_operations(1)?;
                let mut y_polygon = x_polygon;
                let y_len = clip_cell(&mut y_polygon, x_len, 1, y, control)?;
                if y_len < 3 {
                    continue;
                }
                let (z_lo, z_hi) = axis_bounds(&y_polygon[..y_len], 2);
                for z in z_lo..=z_hi {
                    control.charge_operations(1)?;
                    let mut polygon = y_polygon;
                    let len = clip_cell(&mut polygon, y_len, 2, z, control)?;
                    if len < 3 {
                        continue;
                    }
                    let role = usize::from(
                        cell_hash(DVec3::new(x as f64 + 0.5, y as f64 + 0.5, z as f64 + 0.5))? & 8
                            != 0,
                    );
                    for i in 1..len - 1 {
                        control.charge_operations(1)?;
                        let vertices = [polygon[0], polygon[i], polygon[i + 1]];
                        let area = (vertices[1].point - vertices[0].point)
                            .cross(vertices[2].point - vertices[0].point)
                            .length()
                            * 0.5;
                        if !area.is_finite() {
                            return Err(GeometryStop::Numerical);
                        }
                        if area > 0. {
                            visit(
                                role,
                                CellFacet {
                                    vertices,
                                    normal: normals.then_some(normal),
                                    triangle,
                                    area,
                                },
                            )?;
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

#[allow(clippy::cast_possible_truncation)]
fn axis_bounds(polygon: &[Vertex], axis: usize) -> (i64, i64) {
    let (low, high) = polygon
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), vertex| {
            (low.min(vertex.point[axis]), high.max(vertex.point[axis]))
        });
    (low.floor() as i64, high.floor() as i64)
}

fn clip_cell(
    polygon: &mut [Vertex; 12],
    len: usize,
    axis: usize,
    coordinate: i64,
    control: &GeometryControl,
) -> Result<usize, GeometryStop> {
    if axis >= 3 {
        return Err(GeometryStop::Numerical);
    }
    if len > polygon.len() {
        return Err(GeometryStop::ResourceLimit);
    }
    if len == 0 {
        return Ok(0);
    }
    #[allow(clippy::cast_precision_loss)]
    let plane = coordinate as f64;
    let mut lower = [false; 12];
    let mut upper = [false; 12];
    let mut all_lower = true;
    let mut all_upper = true;
    // One classification pass covers both planes. Each tiny edge run remains
    // bounded, and no vertex is reconstructed for a satisfied half-space.
    control.charge_operations(u64::try_from(len).map_err(|_| GeometryStop::ResourceLimit)?)?;
    for (i, vertex) in polygon[..len].iter().enumerate() {
        lower[i] = vertex.point[axis] >= plane;
        upper[i] = vertex.point[axis] <= plane + 1.;
        all_lower &= lower[i];
        all_upper &= upper[i];
    }
    let len = if all_lower {
        len
    } else {
        clip(polygon, len, axis, plane, &lower)?
    };
    // A lower-plane intersection lies inside the upper half-space whenever
    // both original endpoints do. Clipping cannot invalidate this proof.
    if all_upper || len == 0 {
        return Ok(len);
    }
    control.charge_operations(u64::try_from(len).map_err(|_| GeometryStop::ResourceLimit)?)?;
    if !all_lower {
        for (i, vertex) in polygon[..len].iter().enumerate() {
            upper[i] = vertex.point[axis] <= plane + 1.;
        }
    }
    clip(polygon, len, axis, plane + 1., &upper)
}

fn clip(
    polygon: &mut [Vertex; 12],
    len: usize,
    axis: usize,
    plane: f64,
    inside: &[bool; 12],
) -> Result<usize, GeometryStop> {
    let mut output = [Vertex::default(); 12];
    let mut count = 0;
    let mut previous = polygon[len - 1];
    let mut previous_inside = inside[len - 1];
    for (i, &current) in polygon[..len].iter().enumerate() {
        let current_inside = inside[i];
        if previous_inside != current_inside {
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
        if current_inside {
            *output.get_mut(count).ok_or(GeometryStop::ResourceLimit)? = current;
            count += 1;
        }
        previous = current;
        previous_inside = current_inside;
    }
    *polygon = output;
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Soup;

    #[test]
    fn contained_cell_classification_avoids_two_polygon_reconstructions() {
        for axis in 0..3 {
            for coordinate in -2..=2i64 {
                let mut polygon = [Vertex::default(); 12];
                for (i, offset) in [0.1, 0.4, 0.9].into_iter().enumerate() {
                    polygon[i] = Vertex {
                        point: DVec3::new(0.2, 0.3, 0.4),
                        barycentric: [DVec3::X, DVec3::Y, DVec3::Z][i],
                    };
                    #[allow(clippy::cast_precision_loss)]
                    {
                        polygon[i].point[axis] = coordinate as f64 + offset;
                    }
                }
                let expected = polygon[..3]
                    .iter()
                    .map(|v| (v.point, v.barycentric))
                    .collect::<Vec<_>>();
                let control = GeometryControl::unlimited();
                let len = clip_cell(&mut polygon, 3, axis, coordinate, &control).unwrap();
                assert_eq!(len, 3);
                assert_eq!(
                    polygon[..len]
                        .iter()
                        .map(|v| (v.point, v.barycentric))
                        .collect::<Vec<_>>(),
                    expected
                );
                assert_eq!(control.counters().operations, 3);
            }
        }
    }

    #[test]
    fn one_sided_cell_cut_keeps_exact_barycentric_vertices() {
        let mut polygon = [Vertex::default(); 12];
        for (i, point) in [
            DVec3::new(-0.5, 0., 0.),
            DVec3::new(0.5, 0., 0.),
            DVec3::new(0.5, 1., 0.),
        ]
        .into_iter()
        .enumerate()
        {
            polygon[i] = Vertex {
                point,
                barycentric: [DVec3::X, DVec3::Y, DVec3::Z][i],
            };
        }
        let control = GeometryControl::unlimited();
        let len = clip_cell(&mut polygon, 3, 0, 0, &control).unwrap();
        assert_eq!(len, 4);
        assert_eq!(
            polygon[..len]
                .iter()
                .map(|v| (v.point, v.barycentric))
                .collect::<Vec<_>>(),
            [
                (DVec3::new(0., 0.5, 0.), DVec3::new(0.5, 0., 0.5)),
                (DVec3::ZERO, DVec3::new(0.5, 0.5, 0.)),
                (DVec3::new(0.5, 0., 0.), DVec3::Y),
                (DVec3::new(0.5, 1., 0.), DVec3::Z),
            ]
        );
        assert_eq!(control.counters().operations, 3);
    }

    #[test]
    fn broad_surface_populations_fit_without_retaining_cell_fragments() {
        let positions = [0., 0., 0., 300., 0., 0., 300., 300., 0., 0., 300., 0.];
        let index = SurfaceIndex::build(Soup {
            positions: &positions,
            indices: &[0, 1, 2, 0, 2, 3],
            mask: None,
        })
        .unwrap();
        let control = GeometryControl::new(
            crate::CancelFlag::new(),
            std::time::Duration::from_secs(10),
            occluview_geometry::surface::GeometryLimits {
                memory_bytes: 8 * 1024 * 1024,
                operations: 256_000_000,
                ..occluview_geometry::surface::GeometryLimits::default()
            },
        );
        let population = CellPopulation::build(&index, true, &control).unwrap();
        let training = population
            .samples(SampleRole::Training, 8192, 1, &control)
            .unwrap();
        let holdout = population
            .samples(SampleRole::Holdout, 8192, 2, &control)
            .unwrap();
        let mut oracle = [0.; 2];
        for x in 0..300u32 {
            for y in 0..300u32 {
                let role = usize::from(
                    cell_hash(DVec3::new(f64::from(x) + 0.5, f64::from(y) + 0.5, 0.5)).unwrap() & 8
                        != 0,
                );
                oracle[role] += 1.;
            }
        }
        for (batch, area) in [&training, &holdout].into_iter().zip(oracle) {
            assert!((batch.population_area_mm2 - area).abs() <= 1e-8);
            assert!((batch.represented_area_mm2 - area).abs() <= 1e-8);
            assert!(batch.samples.iter().all(|s| {
                (cell_hash(s.point).unwrap() & 8 != 0) == (batch.role == SampleRole::Holdout)
                    && s.barycentric
                        .iter()
                        .all(|&b| (-1e-12..=1. + 1e-12).contains(&b))
            }));
        }
        assert!(control.counters().peak_memory_bytes <= 8 * 1024 * 1024);
        drop(training);
        drop(holdout);
        assert_eq!(control.counters().memory_bytes, 0);
    }

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
        assert_eq!(control.counters().memory_bytes, 0);
    }
}
