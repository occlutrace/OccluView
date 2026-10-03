//! Connected area moments for translation proposals. Geodesic neighbourhoods
//! follow Dijkstra (1959), <https://doi.org/10.1007/BF01386390>, independently
//! expressed over the existing exact triangle surface. Edges join exactly
//! shared corner pairs; disconnected components are never bridged by proximity.
//! Farthest area representatives initialize bounded overlapping neighbourhoods.

use crate::{PreparedSurface, SurfaceSample};
use glam::DVec3;
use occluview_geometry::surface::{GeometryControl, GeometryStop};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BinaryHeap};

/// Overlapping 30%-area windows fit the connected-neighbourhood work ceiling.
pub(crate) const TRANSLATION_PATCH_FRACTION: f64 = 0.30;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Patch {
    pub center: DVec3,
    pub axes: Option<[DVec3; 3]>,
    pub complete: bool,
}

pub(crate) struct PatchSet {
    pub(crate) translations: Vec<Patch>,
    pub(crate) components: Vec<Patch>,
}

#[derive(Clone, Copy)]
struct Node {
    sample: SurfaceSample,
    neighbours: [Option<usize>; 3],
}
#[derive(Clone, Copy, PartialEq)]
struct Frontier {
    distance: f64,
    slot: usize,
}
impl Eq for Frontier {}
impl Ord for Frontier {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .distance
            .total_cmp(&self.distance)
            .then_with(|| other.slot.cmp(&self.slot))
    }
}
impl PartialOrd for Frontier {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Four overlapping quantile windows of 25–50% of smaller eligible area.
/// Source-order representatives carry area weights, rather than vertex counts.
pub(crate) fn smaller_patches(
    surface: &PreparedSurface,
    control: &GeometryControl,
) -> Result<Vec<Patch>, GeometryStop> {
    let samples = &surface.samples[0].samples;
    let mut patches = Vec::with_capacity(4);
    for i in 0..4 {
        let start = i * (samples.len() * 7 / 10) / 3;
        let end = (start + samples.len() * 3 / 10).min(samples.len());
        if let Some((center, axes)) =
            crate::proposal_geometry::principal_frame(&samples[start..end], control)?
        {
            patches.push(Patch {
                center,
                axes: Some(axes),
                complete: true,
            });
        }
    }
    Ok(patches)
}

/// Connected neighbourhood area equal to a smaller-side quantile patch.
/// Graph work is charged before each edge visit and capped across all patches.
/// A deficient disconnected neighbourhood retains its seed with incomplete
/// translation coverage; it cannot claim the requested area was reached.
#[expect(
    clippy::too_many_lines,
    clippy::too_many_arguments,
    reason = "one bounded graph lifetime and patch scratch reservation"
)]
pub(crate) fn larger_patches(
    surface: &PreparedSurface,
    target_area: f64,
    count: usize,
    edge_cap: u64,
    edge_visits: &mut u64,
    control: &GeometryControl,
) -> Result<PatchSet, GeometryStop> {
    let triangle_count = surface.original_index.triangle_count();
    let _memory = control.reserve(
        triangle_count
            .checked_mul(640)
            .ok_or(GeometryStop::ResourceLimit)?,
    )?;
    let mut nodes = Vec::new();
    nodes
        .try_reserve_exact(triangle_count)
        .map_err(|_| GeometryStop::ResourceLimit)?;
    let mut edges = BTreeMap::new();
    for (triangle, corners, normal) in surface.original_index.triangles() {
        control.charge_operations(1)?;
        let slot = nodes.len();
        nodes.push(Node {
            sample: SurfaceSample {
                id: triangle,
                point: corners[0] / 3. + corners[1] / 3. + corners[2] / 3.,
                normal: Some(normal),
                triangle,
                barycentric: [1. / 3.; 3],
                area_weight_mm2: (corners[1] - corners[0])
                    .cross(corners[2] - corners[0])
                    .length()
                    * 0.5,
                region_id: 0,
            },
            neighbours: [None; 3],
        });
        for (local, (a, b)) in [(0, 1), (1, 2), (2, 0)].into_iter().enumerate() {
            control.charge_operations(1)?;
            let key = |p: DVec3| p.to_array().map(|x| if x == 0. { 0 } else { x.to_bits() });
            let first = key(corners[a]);
            let second = key(corners[b]);
            let edge = if first < second {
                (first, second)
            } else {
                (second, first)
            };
            if let Some(&(other, other_edge)) = edges.get(&edge) {
                nodes[slot].neighbours[local] = Some(other);
                nodes[other].neighbours[other_edge] = Some(slot);
            } else {
                edges.insert(edge, (slot, local));
            }
        }
    }
    drop(edges);
    let mut seeds = Vec::new();
    seeds
        .try_reserve_exact(count.min(48))
        .map_err(|_| GeometryStop::ResourceLimit)?;
    let samples = &surface.samples[0].samples;
    let mut nearest = Vec::new();
    nearest
        .try_reserve_exact(samples.len())
        .map_err(|_| GeometryStop::ResourceLimit)?;
    nearest.resize(samples.len(), f64::INFINITY);
    let mut chosen = 0;
    for _ in 0..count.min(48).min(samples.len()) {
        let p = samples[chosen].point;
        seeds.push(p);
        let mut farthest = 0.;
        for (i, s) in samples.iter().enumerate() {
            control.charge_point_pairs(1)?;
            nearest[i] = nearest[i].min(p.distance_squared(s.point));
            if nearest[i] > farthest {
                farthest = nearest[i];
                chosen = i;
            }
        }
    }
    let mut patches = Vec::new();
    patches
        .try_reserve_exact(seeds.len())
        .map_err(|_| GeometryStop::ResourceLimit)?;
    let mut distances = Vec::new();
    distances
        .try_reserve_exact(nodes.len())
        .map_err(|_| GeometryStop::ResourceLimit)?;
    distances.resize(nodes.len(), f64::INFINITY);
    let mut population = Vec::new();
    population
        .try_reserve_exact(nodes.len())
        .map_err(|_| GeometryStop::ResourceLimit)?;
    // Complete connected-component moments preserve disconnected fitting
    // alternatives. Source order fixes traversal and eigenframe ties.
    let mut components = Vec::new();
    components
        .try_reserve_exact(8)
        .map_err(|_| GeometryStop::ResourceLimit)?;
    let mut visited = Vec::new();
    visited
        .try_reserve_exact(nodes.len())
        .map_err(|_| GeometryStop::ResourceLimit)?;
    visited.resize(nodes.len(), false);
    let mut pending = Vec::new();
    pending
        .try_reserve_exact(nodes.len())
        .map_err(|_| GeometryStop::ResourceLimit)?;
    for root in 0..nodes.len() {
        control.charge_operations(1)?;
        if visited[root] || components.len() >= 8 {
            continue;
        }
        if *edge_visits >= edge_cap {
            break;
        }
        pending.clear();
        population.clear();
        pending.push(root);
        visited[root] = true;
        let mut complete = true;
        while let Some(slot) = pending.pop() {
            control.charge_operations(1)?;
            population.push(nodes[slot].sample);
            for other in nodes[slot].neighbours.into_iter().flatten() {
                if *edge_visits >= edge_cap {
                    complete = false;
                    break;
                }
                control.charge_operations(1)?;
                *edge_visits += 1;
                if !visited[other] {
                    visited[other] = true;
                    pending.push(other);
                }
            }
            if !complete {
                break;
            }
        }
        if let Some((center, axes)) =
            crate::proposal_geometry::principal_frame(&population, control)?
        {
            components.push(Patch {
                center,
                axes: Some(axes),
                complete,
            });
        }
        if !complete {
            break;
        }
    }
    drop(visited);
    drop(pending);
    let mut frontier = BinaryHeap::new();
    frontier
        .try_reserve(nodes.len().saturating_mul(3))
        .map_err(|_| GeometryStop::ResourceLimit)?;
    for seed in seeds {
        if *edge_visits >= edge_cap {
            patches.push(Patch {
                center: seed,
                axes: None,
                complete: false,
            });
            continue;
        }
        let mut start = 0;
        let mut best = f64::INFINITY;
        for (slot, node) in nodes.iter().enumerate() {
            control.charge_point_pairs(1)?;
            let distance = seed.distance_squared(node.sample.point);
            if distance < best {
                best = distance;
                start = slot;
            }
        }
        if nodes.is_empty() {
            break;
        }
        // Clear in bounded blocks so cancellation cannot hide in a large fill.
        for block in distances.chunks_mut(128) {
            control.charge_operations(block.len() as u64)?;
            block.fill(f64::INFINITY);
        }
        population.clear();
        frontier.clear();
        distances[start] = 0.;
        frontier.push(Frontier {
            distance: 0.,
            slot: start,
        });
        let mut area = 0.;
        while let Some(Frontier { distance, slot }) = frontier.pop() {
            control.charge_operations(1)?;
            if distance.to_bits() != distances[slot].to_bits() {
                continue;
            }
            let mut sample = nodes[slot].sample;
            sample.area_weight_mm2 = sample.area_weight_mm2.min((target_area - area).max(0.));
            area += sample.area_weight_mm2;
            population.push(sample);
            if area >= target_area || *edge_visits >= edge_cap {
                break;
            }
            for other in nodes[slot].neighbours.into_iter().flatten() {
                if *edge_visits >= edge_cap {
                    break;
                }
                control.charge_operations(1)?;
                *edge_visits += 1;
                let next = distance + nodes[slot].sample.point.distance(nodes[other].sample.point);
                if next < distances[other] {
                    distances[other] = next;
                    frontier.push(Frontier {
                        distance: next,
                        slot: other,
                    });
                }
            }
        }
        let complete = area >= target_area;
        let frame = crate::proposal_geometry::principal_frame(&population, control)?;
        patches.push(Patch {
            center: if complete {
                frame.map_or(seed, |f| f.0)
            } else {
                seed
            },
            axes: frame.map(|f| f.1),
            complete,
        });
    }
    Ok(PatchSet {
        translations: patches,
        components,
    })
}
