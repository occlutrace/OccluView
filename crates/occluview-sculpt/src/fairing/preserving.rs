//! Shape-preserving implicit smoothing: two membrane solves combined so that
//! a round form keeps its shape while the bumps below the scale go. Each
//! bounded solve keeps input selection order and starts at the current surface.

use super::*;
use crate::hash::FxHashMap;

/// Residual share each solve stops at. A brush step applies a share of the
/// filtered surface; the next step solves again from that surface.
const BRUSH_TOLERANCE: f64 = 1e-2;
/// Bound the work spent on each solve. Remaining correction is picked up by
/// later calls, which start from the latest live surface.
const BRUSH_SOLVE_STEPS: usize = 48;

/// Fair one selection with the shape-preserving filter and report where each
/// vertex should land. The contract is `fair_selection`'s.
pub fn fair_selection_preserving<S: FairingSurface + ?Sized>(
    surface: &S,
    selection: &[(u32, f64)],
    feature_size_mm: f64,
    scratch: &mut FairingScratch,
    out: &mut Vec<(u32, DVec3)>,
) {
    out.clear();
    let whole: Vec<(u32, f64)> = selection
        .iter()
        .map(|&(vertex, weight)| (vertex, if weight > 0.0 { 1.0 } else { 0.0 }))
        .collect();
    let once = fair_selection_within(
        surface,
        &whole,
        feature_size_mm,
        scratch,
        FairingSolveBounds {
            tolerance: BRUSH_TOLERANCE,
            steps: BRUSH_SOLVE_STEPS,
        },
    );
    if once.is_empty() {
        return;
    }
    let smoothed = Smoothed {
        surface,
        moved: once.iter().copied().collect(),
    };
    let twice = fair_selection_within(
        &smoothed,
        &whole,
        feature_size_mm,
        scratch,
        FairingSolveBounds {
            tolerance: BRUSH_TOLERANCE,
            steps: BRUSH_SOLVE_STEPS,
        },
    );
    let twice: FxHashMap<u32, DVec3> = twice.into_iter().collect();
    for &(vertex, weight) in selection {
        if !(weight > 0.0) {
            continue;
        }
        let here = surface.position(vertex);
        let first = smoothed.position(vertex);
        let second = twice.get(&vertex).copied().unwrap_or(first);
        let solved = first * 2.0 - second;
        let target = here + (solved - here) * weight;
        if target.is_finite() && (target - here).length() > 1e-15 {
            out.push((vertex, target));
        }
    }
}

/// The surface with the first solve's positions in place of the live ones.
struct Smoothed<'a, S: FairingSurface + ?Sized> {
    surface: &'a S,
    moved: FxHashMap<u32, DVec3>,
}

impl<S: FairingSurface + ?Sized> FairingSurface for Smoothed<'_, S> {
    fn vertex_count(&self) -> usize {
        self.surface.vertex_count()
    }

    fn position(&self, vertex: u32) -> DVec3 {
        self.moved
            .get(&vertex)
            .copied()
            .unwrap_or_else(|| self.surface.position(vertex))
    }

    fn vertex_area(&self, vertex: u32) -> f64 {
        self.surface.vertex_area(vertex)
    }

    fn neighbors(&self, vertex: u32) -> &[u32] {
        self.surface.neighbors(vertex)
    }
}
