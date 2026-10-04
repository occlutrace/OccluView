//! Cap fairing: second-order umbrella relaxation (Kobbelt et al. 1998, the
//! fairing step of Liepa 2003 "Filling Holes in Meshes").
//!
//! The refined cap from `cap_refine` already has a good global shape (quadric
//! base + harmonic rim residual), but its seam still meets the surrounding
//! surface with a small slope jump: the quadric is a least-squares blend, not
//! an interpolant of the outside curvature. Fairing polishes exactly that —
//! interior vertices relax toward `U²(v) = 0` (discrete thin-plate) while the
//! rim and its first ring outside the rim stay fixed, so the solution
//! continues the surrounding surface's tangent plane across the seam instead
//! of merely touching its positions.
//!
//! Solved by Kobbelt's local iteration: `v_i ← v_i − U²(v_i) / ν_i` with
//! `ν_i = 1 + (1/n_i) Σ_j 1/n_j`, uniform umbrella weights, deterministic
//! sweep order, tolerance-based early exit with a hard sweep bound.

use crate::numeric::count_as_f32;
use glam::Vec3;

/// Hard bound on fairing sweeps (safety valve; tolerance exits earlier).
const MAX_FAIR_SWEEPS: usize = 160;
/// Convergence: stop when no vertex moved more than this fraction of the
/// cap's bounding-sphere-ish scale during a sweep.
const FAIR_TOLERANCE_FACTOR: f32 = 1e-5;

/// One rim vertex's fixed surroundings: the positions of its mesh neighbors
/// outside the cap. They complete the rim vertex's full umbrella so `U(rim)`
/// measures the real surface, letting the interior blend curvature across the
/// seam.
pub(super) struct RimSupport {
    /// Positions of the rim vertex's outside (non-cap) neighbors.
    pub(super) outside: Vec<Vec3>,
}

/// Largest total normal rotation fairing may apply to any incident triangle,
/// measured against the triangle's normal at the start of fairing (cosine of
/// 45°). Thin-plate continuation of steep boundary slopes (a funnel-like
/// outside ring) has no fold-free solution — the raw Kobbelt iteration then
/// walks interior triangles right through a fold, one small step at a time, and
/// the downstream `candidate_folds` guard refuses the whole cap. Guarding the
/// accumulated rotation keeps every pairwise cap dihedral well under that
/// guard's 120° limit (two adjacent triangles can drift at most 45° each in
/// opposite directions), so fairing always yields an emittable cap.
const TOTAL_NORMAL_COSINE_LIMIT: f32 = std::f32::consts::FRAC_1_SQRT_2; // 45°
/// Step halvings tried before a vertex sits a sweep out.
const STEP_DAMPINGS: usize = 2;

/// Fair the interior of a cap in place. `positions` holds all cap vertices
/// (`0..rim_len` = rim, fixed; the rest = generated interior, moved).
/// `support[i]` belongs to rim vertex `i` and must have `rim_len` entries.
pub(super) fn fair_cap_interior(
    positions: &mut [Vec3],
    rim_len: usize,
    triangles: &[[usize; 3]],
    support: &[RimSupport],
) {
    let vertex_count = positions.len();
    if vertex_count <= rim_len || support.len() != rim_len {
        return;
    }

    let (neighbors, incident) = cap_connectivity(triangles, vertex_count);

    // Full valence per vertex: cap neighbors plus, for rim vertices, their
    // fixed outside neighbors.
    let valence: Vec<f32> = (0..vertex_count)
        .map(|index| {
            let outside = if index < rim_len {
                support[index].outside.len()
            } else {
                0
            };
            count_as_f32(neighbors[index].len() + outside)
        })
        .collect();

    let tolerance = cap_scale(positions) * FAIR_TOLERANCE_FACTOR;
    let mut umbrella: Vec<Vec3> = vec![Vec3::ZERO; vertex_count];
    let guard = FoldGuard::at_start(positions, triangles);

    for _ in 0..MAX_FAIR_SWEEPS {
        // U(v) for every cap vertex, rim included (its umbrella spans the
        // outside neighbors, which is what carries curvature across the seam).
        for index in 0..vertex_count {
            let mut sum = Vec3::ZERO;
            let mut count = neighbors[index].len();
            for &neighbor in &neighbors[index] {
                sum += positions[neighbor];
            }
            if index < rim_len {
                for outside in &support[index].outside {
                    sum += *outside;
                }
                count += support[index].outside.len();
            }
            umbrella[index] = if count == 0 {
                Vec3::ZERO
            } else {
                sum / count_as_f32(count) - positions[index]
            };
        }

        // Kobbelt update on interior vertices only, fold-guarded: a step is
        // accepted (possibly damped) only while no incident triangle's normal
        // rotates past the per-step limit.
        let mut max_move = 0.0_f32;
        for index in rim_len..vertex_count {
            let ring = &neighbors[index];
            if ring.is_empty() || valence[index] <= 0.0 {
                continue;
            }
            let mut umbrella_mean = Vec3::ZERO;
            let mut nu = 1.0_f32;
            for &neighbor in ring {
                umbrella_mean += umbrella[neighbor];
                if valence[neighbor] > 0.0 {
                    nu += (1.0 / valence[neighbor]) / valence[index];
                }
            }
            umbrella_mean /= valence[index];
            let second_order = umbrella_mean - umbrella[index];
            let mut step = second_order / nu;
            if !step.is_finite() {
                continue;
            }
            for _ in 0..=STEP_DAMPINGS {
                let proposed = positions[index] - step;
                if guard.step_keeps_normals(positions, &incident[index], index, proposed) {
                    positions[index] = proposed;
                    max_move = max_move.max(step.length());
                    break;
                }
                step *= 0.5;
            }
        }
        if max_move <= tolerance {
            break;
        }
    }
}

/// Cap connectivity (insertion-ordered, deterministic): vertex neighbors and
/// per-vertex incident triangles. Out-of-range indices are skipped.
#[allow(clippy::type_complexity)] // Two parallel adjacency tables, one build.
fn cap_connectivity(
    triangles: &[[usize; 3]],
    vertex_count: usize,
) -> (Vec<Vec<usize>>, Vec<Vec<usize>>) {
    let mut neighbors: Vec<Vec<usize>> = vec![Vec::new(); vertex_count];
    let mut incident: Vec<Vec<usize>> = vec![Vec::new(); vertex_count];
    for (triangle_index, &[a, b, c]) in triangles.iter().enumerate() {
        for (u, v) in [(a, b), (b, c), (c, a)] {
            if u >= vertex_count || v >= vertex_count {
                continue;
            }
            if !neighbors[u].contains(&v) {
                neighbors[u].push(v);
            }
            if !neighbors[v].contains(&u) {
                neighbors[v].push(u);
            }
        }
        if a < vertex_count && b < vertex_count && c < vertex_count {
            for &vertex in &[a, b, c] {
                incident[vertex].push(triangle_index);
            }
        }
    }
    (neighbors, incident)
}

/// The anti-fold step gate: triangle normals captured at the start of fairing
/// (the lifted quadric+residual cap is smooth and fold-free), against which
/// every proposed vertex move is checked. Accumulated rotation measures the
/// fold directly; a per-step check lets a fold build up gradually.
struct FoldGuard<'a> {
    triangles: &'a [[usize; 3]],
    initial_normals: Vec<Vec3>,
}

impl<'a> FoldGuard<'a> {
    fn at_start(positions: &[Vec3], triangles: &'a [[usize; 3]]) -> Self {
        let vertex_count = positions.len();
        let initial_normals = triangles
            .iter()
            .map(|&[a, b, c]| {
                if a < vertex_count && b < vertex_count && c < vertex_count {
                    shape_normal(positions[a], positions[b], positions[c])
                } else {
                    Vec3::ZERO
                }
            })
            .collect();
        Self {
            triangles,
            initial_normals,
        }
    }

    /// Whether moving `moved` to `proposed` keeps every incident triangle's
    /// normal within [`TOTAL_NORMAL_COSINE_LIMIT`] of its direction at the
    /// start of fairing. Triangles whose initial normal is degenerate accept
    /// any move; a move that collapses a healthy triangle is refused.
    fn step_keeps_normals(
        &self,
        positions: &[Vec3],
        incident: &[usize],
        moved: usize,
        proposed: Vec3,
    ) -> bool {
        if !proposed.is_finite() {
            return false;
        }
        for &triangle_index in incident {
            let triangle = self.triangles[triangle_index];
            let at = |index: usize| -> Vec3 {
                if index == moved {
                    proposed
                } else {
                    positions[index]
                }
            };
            let initial = self.initial_normals[triangle_index];
            let initial_length = initial.length();
            if initial_length <= f32::EPSILON {
                continue;
            }
            let after = {
                let [a, b, c] = triangle.map(at);
                shape_normal(a, b, c)
            };
            let after_length = after.length();
            if after_length <= f32::EPSILON {
                return false;
            }
            if initial.dot(after) / (initial_length * after_length) < TOTAL_NORMAL_COSINE_LIMIT {
                return false;
            }
        }
        true
    }
}

/// Triangle normal of the edges divided by their largest coordinate first.
/// The length is then a unit-free shape measure (about twice the area over the
/// squared longest edge), so the degeneracy tests do not depend on the model's
/// size, and no coordinate magnitude can overflow or underflow the cross
/// product. Zero for a collapsed or non-finite triangle.
fn shape_normal(a: Vec3, b: Vec3, c: Vec3) -> Vec3 {
    let (ab, ac) = (b - a, c - a);
    let extent = ab.abs().max_element().max(ac.abs().max_element());
    if !extent.is_finite() || extent <= 0.0 {
        return Vec3::ZERO;
    }
    let normal = (ab / extent).cross(ac / extent);
    if normal.is_finite() {
        normal
    } else {
        Vec3::ZERO
    }
}

/// A representative geometric scale for the cap: half the bounding-box
/// diagonal (used only to make the convergence tolerance dimensionful).
fn cap_scale(positions: &[Vec3]) -> f32 {
    let mut lo = Vec3::splat(f32::MAX);
    let mut hi = Vec3::splat(f32::MIN);
    for &p in positions {
        lo = lo.min(p);
        hi = hi.max(p);
    }
    let diagonal = (hi - lo).length();
    if diagonal.is_finite() && diagonal > 0.0 {
        diagonal * 0.5
    } else {
        1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cap_support::rim_outside_support;
    use crate::{EditVertex, MeshEditBuffers, MeshTopology};

    #[test]
    fn fairing_fold_guard_is_scale_independent() {
        let triangles = [[0, 1, 2]];
        for scale in [0.015, 1.0e-20, 1.0e-4, 1.0, 1.0e20] {
            let positions = [Vec3::ZERO, Vec3::X * scale, Vec3::Y * scale];
            let guard = FoldGuard::at_start(&positions, &triangles);
            assert!(guard.step_keeps_normals(&positions, &[0], 2, Vec3::Y * scale));
            assert!(
                !guard.step_keeps_normals(&positions, &[0], 2, -Vec3::Y * scale),
                "an inverted facet must be refused at scale {scale}"
            );
            assert!(
                !guard.step_keeps_normals(&positions, &[0], 2, Vec3::ZERO),
                "a healthy facet must not collapse at scale {scale}"
            );
            assert!(!guard.step_keeps_normals(&positions, &[0], 2, Vec3::splat(f32::NAN)));
        }
    }

    #[test]
    fn cap_fairing_mean_keeps_large_outside_valence_and_translation_invariance() {
        let count = 70_000;
        let mut vertices = vec![
            EditVertex::at(Vec3::ZERO.to_array()),
            EditVertex::at(Vec3::X.to_array()),
            EditVertex::at(Vec3::Y.to_array()),
        ];
        vertices.extend((0..count).map(|index| {
            EditVertex::at(if index % 2 == 0 {
                [2.0, 0.0, 1.0]
            } else {
                [2.0, 1.0, 0.0]
            })
        }));
        let indices = (0..count)
            .flat_map(|index| {
                [
                    0,
                    u32::try_from(index + 3).expect("bounded vertex"),
                    u32::try_from((index + 1) % count + 3).expect("bounded vertex"),
                ]
            })
            .collect();
        let mesh = MeshEditBuffers {
            vertices,
            indices,
            topology: MeshTopology::TriangleMesh,
        };
        let adjacency = vec![(3..count + 3).collect(), vec![], vec![]];
        let support = rim_outside_support(&mesh, &[0, 1, 2], &adjacency);
        assert_eq!(support[0].outside.len(), count);
        let mut original = vec![Vec3::ZERO, Vec3::X, Vec3::Y, (Vec3::X + Vec3::Y) / 3.0];
        let mut translated = original
            .iter()
            .map(|point| *point + Vec3::splat(4.0))
            .collect::<Vec<_>>();
        let shifted = support
            .iter()
            .map(|ring| RimSupport {
                outside: ring
                    .outside
                    .iter()
                    .map(|point| *point + Vec3::splat(4.0))
                    .collect(),
            })
            .collect::<Vec<_>>();
        let triangles = [[0, 1, 3], [1, 2, 3], [2, 0, 3]];
        fair_cap_interior(&mut original, 3, &triangles, &support);
        fair_cap_interior(&mut translated, 3, &triangles, &shifted);
        assert!(original[3].distance(translated[3] - Vec3::splat(4.0)) < 1.0e-4,
            "outside support averages must not depend on the position of the mesh: {original:?} vs {translated:?}");
    }
}
