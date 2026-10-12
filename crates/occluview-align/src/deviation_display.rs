//! The map as the operator sees it.
//!
//! Two scans of one arch never agree to the last micron: each carries its own
//! sensor noise, so the distance from one vertex to the other surface jitters
//! from vertex to vertex by a few hundredths of a millimetre. Painted straight
//! from the ramp, that jitter is a mosaic of single-vertex speckles whose
//! colour changes with the slightest move, and it hides the structure the map
//! exists to show — where one scan sits systematically off the other.
//!
//! The display therefore paints a lightly smoothed copy. The statistics, the
//! scale and the pass/fail numbers keep reading the raw map; only the colours
//! are calmed.

use occluview_geometry::coincident_position_key;
use rayon::prelude::*;

use crate::{CancelFlag, DeviationMap, Soup, Validity};

/// Neighbour-averaging passes applied to the displayed map.
///
/// An intraoral scan has a vertex every tenth of a millimetre or so, so three
/// passes reach about a third of a millimetre: enough to calm single-vertex
/// noise while keeping narrow transitions in the measurement visible.
pub const DISPLAY_PASSES: usize = 3;

/// A copy of `map` with each measured value averaged with its measured
/// neighbours [`DISPLAY_PASSES`] times.
///
/// Neighbours are the vertices a triangle edge joins, after vertices at one
/// position are taken as one: an STL is a soup of per-triangle corners, and
/// without that a corner would have no neighbours at all. Unmeasured vertices
/// are neither changed nor allowed to pull a measured one toward them, so the
/// edge of the measured region keeps its values.
///
/// A map that does not match the geometry is returned as it is.
#[must_use]
pub fn display_map(map: &DeviationMap, moving: Soup<'_>) -> DeviationMap {
    display_map_cancellable(map, moving, &CancelFlag::new()).unwrap_or_else(|| map.clone())
}

/// Build the softened display copy, stopping between expensive topology and
/// smoothing stages when the owning measurement is no longer current.
#[must_use]
pub fn display_map_cancellable(
    map: &DeviationMap,
    moving: Soup<'_>,
    cancel: &CancelFlag,
) -> Option<DeviationMap> {
    if cancel.is_cancelled() {
        return None;
    }
    let count = moving.vertex_count();
    if map.signed_mm.len() != count || map.validity.len() != count || count == 0 {
        let copy = map.clone();
        return (!cancel.is_cancelled()).then_some(copy);
    }
    let sites = Sites::weld(moving, map, cancel)?;
    let mut shown = sites.values.clone();
    let mut next = shown.clone();
    for _ in 0..DISPLAY_PASSES {
        next.par_iter_mut().enumerate().for_each(|(site, out)| {
            if cancel.is_cancelled() || !sites.live[site] {
                return;
            }
            let mut sum = f64::from(shown[site]);
            let mut members = 1.0_f64;
            for &other in sites.neighbours(site) {
                let other = other as usize;
                if sites.live[other] {
                    sum += f64::from(shown[other]);
                    members += 1.0;
                }
            }
            #[allow(clippy::cast_possible_truncation)]
            {
                *out = (sum / members) as f32;
            }
        });
        if cancel.is_cancelled() {
            return None;
        }
        std::mem::swap(&mut shown, &mut next);
    }
    let displayed = DeviationMap {
        signed_mm: map
            .signed_mm
            .iter()
            .zip(&sites.site_of)
            .zip(&map.validity)
            .map(|((raw, &site), state)| {
                if *state == Validity::Measured {
                    shown[site as usize]
                } else {
                    *raw
                }
            })
            .collect(),
        validity: map.validity.clone(),
    };
    (!cancel.is_cancelled()).then_some(displayed)
}

/// The distinct positions of a mesh and the edges between them.
struct Sites {
    /// The site each vertex belongs to.
    site_of: Vec<u32>,
    /// The measured value of each site, where it has one.
    values: Vec<f32>,
    /// Whether the site carries a measurement.
    live: Vec<bool>,
    /// Neighbour lists, flattened: site `i` owns `edges[offsets[i]..offsets[i+1]]`.
    offsets: Vec<usize>,
    edges: Vec<u32>,
}

impl Sites {
    fn weld(moving: Soup<'_>, map: &DeviationMap, cancel: &CancelFlag) -> Option<Self> {
        let count = moving.vertex_count();
        let position = |vertex: usize| {
            let at = vertex * 3;
            [
                moving.positions[at],
                moving.positions[at + 1],
                moving.positions[at + 2],
            ]
        };
        let mut order: Vec<([i32; 3], u32)> = (0..count)
            .into_par_iter()
            .map(|vertex| {
                if cancel.is_cancelled() {
                    return ([i32::MIN; 3], u32::MAX);
                }
                (
                    coincident_position_key(position(vertex)),
                    u32::try_from(vertex).unwrap_or(u32::MAX),
                )
            })
            .collect();
        if cancel.is_cancelled() {
            return None;
        }
        order.par_sort_unstable();
        if cancel.is_cancelled() {
            return None;
        }

        if cancel.is_cancelled() {
            return None;
        }
        let mut site_of = vec![0_u32; count];
        let mut values: Vec<f32> = Vec::new();
        let mut live: Vec<bool> = Vec::new();
        let mut previous: Option<[i32; 3]> = None;
        for (ordinal, (key, vertex)) in order.iter().enumerate() {
            if ordinal.is_multiple_of(1024) && cancel.is_cancelled() {
                return None;
            }
            if previous != Some(*key) {
                values.push(0.0);
                live.push(false);
                previous = Some(*key);
            }
            let site = values.len() - 1;
            site_of[*vertex as usize] = u32::try_from(site).unwrap_or(u32::MAX);
            // The first measured member speaks for the site: they sit at one
            // position, so they were measured against the same surface point.
            if !live[site] && map.validity[*vertex as usize] == Validity::Measured {
                live[site] = true;
                values[site] = map.signed_mm[*vertex as usize];
            }
        }

        let sites = values.len();
        let mut pairs: Vec<u64> = moving
            .indices
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|triangle| {
                if cancel.is_cancelled() {
                    return [(None, None), (None, None), (None, None)];
                }
                let corner = |slot: usize| {
                    usize::try_from(triangle[slot])
                        .ok()
                        .and_then(|vertex| site_of.get(vertex).copied())
                };
                let (a, b, c) = (corner(0), corner(1), corner(2));
                [(a, b), (b, c), (c, a)]
            })
            .filter_map(|(from, to)| {
                let (from, to) = (from?, to?);
                (from != to).then(|| (u64::from(from.min(to)) << 32) | u64::from(from.max(to)))
            })
            .collect();
        if cancel.is_cancelled() {
            return None;
        }
        pairs.par_sort_unstable();
        if cancel.is_cancelled() {
            return None;
        }
        pairs.dedup();

        let mut degree = vec![0_usize; sites + 1];
        for (ordinal, pair) in pairs.iter().enumerate() {
            if ordinal.is_multiple_of(1024) && cancel.is_cancelled() {
                return None;
            }
            degree[(pair >> 32) as usize] += 1;
            degree[(pair & 0xFFFF_FFFF) as usize] += 1;
        }
        let mut offsets = vec![0_usize; sites + 1];
        for site in 0..sites {
            if site.is_multiple_of(1024) && cancel.is_cancelled() {
                return None;
            }
            offsets[site + 1] = offsets[site] + degree[site];
        }
        if cancel.is_cancelled() {
            return None;
        }
        let mut cursor = offsets.clone();
        let mut edges = vec![0_u32; offsets[sites]];
        for (ordinal, pair) in pairs.iter().enumerate() {
            if ordinal.is_multiple_of(1024) && cancel.is_cancelled() {
                return None;
            }
            let (low, high) = ((pair >> 32) as usize, (pair & 0xFFFF_FFFF) as usize);
            edges[cursor[low]] = u32::try_from(high).unwrap_or(u32::MAX);
            cursor[low] += 1;
            edges[cursor[high]] = u32::try_from(low).unwrap_or(u32::MAX);
            cursor[high] += 1;
        }
        Some(Self {
            site_of,
            values,
            live,
            offsets,
            edges,
        })
    }

    fn neighbours(&self, site: usize) -> &[u32] {
        &self.edges[self.offsets[site]..self.offsets[site + 1]]
    }
}

#[cfg(test)]
#[path = "deviation_display_tests.rs"]
mod tests;
