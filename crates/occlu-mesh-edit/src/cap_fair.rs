//! Cap fairing: second-order umbrella relaxation (Kobbelt et al. 1998, the
//! fairing step of Liepa 2003 "Filling Holes in Meshes").
//!
//! The refined cap from `cap_refine` already has a good global shape (quadric
//! base + harmonic rim residual), but its seam still meets the surrounding
//! surface with a small slope jump: the quadric is a least-squares blend, not
//! an interpolant of the outside curvature. Fairing polishes exactly that —
//! interior vertices relax toward `U²(v) = 0` (discrete thin-plate) while the
//! rim AND its first ring OUTSIDE the rim stay fixed, so the solution
//! continues the surrounding surface's tangent plane across the seam instead
//! of merely touching its positions.
//!
//! Solved by Kobbelt's local iteration: `v_i ← v_i − U²(v_i) / ν_i` with
//! `ν_i = 1 + (1/n_i) Σ_j 1/n_j`, uniform umbrella weights, deterministic
//! sweep order, tolerance-based early exit with a hard sweep bound.

use glam::Vec3;

/// Hard bound on fairing sweeps (safety valve; tolerance exits earlier).
const MAX_FAIR_SWEEPS: usize = 160;
/// Convergence: stop when no vertex moved more than this fraction of the
/// cap's bounding-sphere-ish scale during a sweep.
const FAIR_TOLERANCE_FACTOR: f32 = 1e-5;

/// One rim vertex's fixed surroundings: the positions of its mesh neighbors
/// OUTSIDE the cap. They complete the rim vertex's full umbrella so `U(rim)`
/// measures the real surface, letting the interior blend curvature across the
/// seam.
pub(super) struct RimSupport {
    /// Positions of the rim vertex's outside (non-cap) neighbors.
    pub(super) outside: Vec<Vec3>,
}

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

    // Cap connectivity (insertion-ordered, deterministic).
    let mut neighbors: Vec<Vec<usize>> = vec![Vec::new(); vertex_count];
    for &[a, b, c] in triangles {
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
    }

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

        // Kobbelt update on interior vertices only.
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
            let step = second_order / nu;
            if step.is_finite() {
                positions[index] -= step;
                max_move = max_move.max(step.length());
            }
        }
        if max_move <= tolerance {
            break;
        }
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

/// Count-to-f32 via `u16` (cap fans never approach `u16::MAX`; saturation
/// only guards a pathological input). Mirrors `cap_refine::count_as_f32`.
fn count_as_f32(count: usize) -> f32 {
    f32::from(u16::try_from(count).unwrap_or(u16::MAX))
}
