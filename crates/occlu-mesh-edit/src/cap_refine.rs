//! Interpolated hole caps (exocad-style "close holes"): refine a planar
//! ear-clip cap with interior vertices until it matches the rim's edge
//! density, then drape the interior onto a smooth surface fitted to the rim so
//! the patch follows the surrounding shape instead of denting inward.
//!
//! All connectivity work is done in the cap's 2D tangent plane, where a
//! Delaunay triangulation is well defined and unique. This matters: refining an
//! ear-clip fan in 3D and flipping toward "max-min angle" leaves a high-valence
//! hub of radiating sliver triangles (a visible starburst). In 2D, iterating
//! Lawson edge flips to convergence reaches the (constrained) Delaunay
//! triangulation — bounded valence, no hub, no slivers.
//!
//! Two-part scheme:
//!  1. Size-driven refinement (Liepa 2003, with Rivara-style splitting):
//!     longest-interior-edge bisection of every triangle larger than the
//!     local target edge scale, with Lawson flips between passes.
//!  2. Curvature-following lift: a quadric height field is least-squares fitted
//!     to the rim AND a band of surface samples just outside it (the rim ring
//!     alone is nearly planar and carries no curvature). Interior vertices are
//!     distributed evenly in-plane, then lifted onto that quadric. Pure
//!     umbrella relaxation in 3D converges to the MINIMAL surface, which sinks
//!     into a visible dent on any curved hole; lifting onto the fitted surface
//!     removes the dent.
//!
//! Rim vertices are never moved and rim edges are never split, so the cap stays
//! a drop-in watertight patch with no T-junctions.

use super::cap_delaunay::{flip_to_delaunay, relax_uv};
use super::{EditVertex, GeneratedVertexPolicy};
use glam::{Vec2, Vec3};

/// Liepa's density factor (√2): an interior edge is bisected while it is
/// longer than this factor times the local target edge scale.
const ALPHA: f32 = std::f32::consts::SQRT_2;
/// Refinement rounds; each round bisects every over-long interior edge once,
/// halving the worst edge, so the pass count needed is logarithmic in
/// (cap diameter / target scale) — 16 covers every practical hole.
const MAX_REFINE_PASSES: usize = 16;
/// Harmonic sweeps that blend the rim residual into the cap interior. The rim
/// residual decays over a few vertex rings, so this needs to be generous enough
/// to propagate across a large cap.
const HARMONIC_ITERATIONS: usize = 128;
/// Hard cap on generated vertices per hole, as a multiple of the rim length.
/// Density is set by the rim edge scale; this is only a runaway safety valve.
const MAX_GENERATED_PER_RIM: usize = 32;
/// Absolute interior-vertex budget per hole. Rim-density refinement of a large
/// hole needs `O(rim_len^2)` interior vertices, which made a 1000-edge rim take
/// seconds and a 20 000-edge rim minutes. When the density estimate exceeds
/// this budget, the target edge scale is raised so refinement terminates at a
/// uniformly coarser (still even) sampling instead of stalling mid-pass.
const CAP_INTERIOR_BUDGET: usize = 12_000;
/// Tikhonov ridge added to the (scale-normalized) quadric normal equations.
const QUADRIC_RIDGE: f32 = 1e-4;

/// A refined cap: generated interior vertices plus the full cap triangulation
/// in LOCAL indices (`0..rim_len` = rim order, `rim_len..` = generated).
pub(super) struct RefinedCap {
    pub(super) generated: Vec<EditVertex>,
    pub(super) triangles: Vec<[usize; 3]>,
}

/// A local orthonormal frame plus a quadric height field over it. Interior cap
/// vertices are lifted onto `centroid + a*u + b*v + height(a,b)*normal`.
struct CapSurface {
    centroid: Vec3,
    u: Vec3,
    v: Vec3,
    normal: Vec3,
    /// Coefficients of `h = c0 + c1 a + c2 b + c3 a^2 + c4 a b + c5 b^2`.
    coeffs: [f32; 6],
}

impl CapSurface {
    /// Local `(a, b)` planar coordinates of a 3D point in this frame.
    fn local_ab(&self, position: Vec3) -> Vec2 {
        let relative = position - self.centroid;
        Vec2::new(relative.dot(self.u), relative.dot(self.v))
    }

    /// Fitted height above the plane at planar coordinates `(a, b)`.
    fn height(&self, ab: Vec2) -> f32 {
        let c = &self.coeffs;
        let (a, b) = (ab.x, ab.y);
        c[0] + c[1] * a + c[2] * b + c[3] * a * a + c[4] * a * b + c[5] * b * b
    }

    /// Lift planar coordinates onto the fitted surface in 3D.
    fn lift(&self, ab: Vec2) -> Vec3 {
        self.centroid + ab.x * self.u + ab.y * self.v + self.height(ab) * self.normal
    }

    /// Lift with an extra height offset above the fitted surface (used to blend
    /// in the harmonically interpolated rim residual).
    fn lift_with(&self, ab: Vec2, extra_height: f32) -> Vec3 {
        self.centroid
            + ab.x * self.u
            + ab.y * self.v
            + (self.height(ab) + extra_height) * self.normal
    }
}

/// Refine and relax an ear-clip cap over `rim` (ring order, 3D positions).
/// `support` holds surface samples just outside the rim (curvature pinning).
/// `initial` holds ear-clip triangles in local rim indices with the final
/// winding already applied by the caller.
pub(super) fn refine_and_relax(
    rim: &[EditVertex],
    support: &[[f32; 3]],
    initial: Vec<[usize; 3]>,
    policy: GeneratedVertexPolicy,
) -> RefinedCap {
    let rim_len = rim.len();
    let rim_positions: Vec<Vec3> = rim
        .iter()
        .map(|vertex| Vec3::from_array(vertex.position))
        .collect();
    let surface = fit_cap_surface(&rim_positions, support);

    // Work in the cap's tangent plane. Rim vertices keep their exact projected
    // coordinates; generated vertices are created and relaxed here.
    let uv: Vec<Vec2> = rim_positions.iter().map(|&p| surface.local_ab(p)).collect();
    let attrs: Vec<EditVertex> = rim.to_vec();
    // Target edge scale per vertex: rim vertices average their two rim edges.
    let mut scale: Vec<f32> = (0..rim_len)
        .map(|index| {
            let prev = uv[(index + rim_len - 1) % rim_len];
            let next = uv[(index + 1) % rim_len];
            let here = uv[index];
            (here.distance(prev) + here.distance(next)) * 0.5
        })
        .collect();
    let mut triangles = initial;
    rescale_for_budget(&uv, rim_len, &mut scale);

    // A first flip sweep turns the ear-clip fan into the Delaunay triangulation
    // of the rim before any splitting, so we densify a clean base.
    flip_to_delaunay(&uv, &mut triangles);

    // Density refinement by LONGEST-INTERIOR-EDGE bisection (Rivara-style),
    // with Lawson flips between passes. Bisection is the sliver-proof choice:
    // the ear-clip base of a many-thousand-edge rim is a fan of long slivers
    // that a bounded number of flip sweeps cannot fully regularize, and
    // centroid (1:3) splits of slivers cascade — a 8000-edge rim used to blow
    // straight to the runaway valve (256k vertices, minutes of work).
    // Halving the longest edge attacks exactly the sliver axis, provably
    // terminates (each split halves one edge, lengths are bounded below by
    // the target scale), and the interleaved flips restore Delaunay quality
    // pass by pass.
    let mut patch = CapPatch {
        uv,
        scale,
        attrs,
        triangles,
    };
    for _ in 0..MAX_REFINE_PASSES {
        let split_any = bisect_pass(&mut patch, rim_len, |ab| surface.lift(ab), policy);
        flip_to_delaunay(&patch.uv, &mut patch.triangles);
        if !split_any {
            break;
        }
    }
    let CapPatch {
        mut uv,
        scale: _,
        attrs,
        triangles,
    } = patch;

    relax_uv(&mut uv, rim_len, &triangles);

    // Blend the quadric base with the rim's residual (its deviation from the
    // quadric). The rim of a wavy surface undulates off the smooth quadric; a
    // pure quadric cap meets it with a slope crease. Harmonically interpolating
    // that residual inward makes the cap meet the rim tangentially, then decays
    // to the quadric in the interior. Only the SMALL residual is harmonic, so
    // no minimal-surface dent is reintroduced.
    let mut residual = vec![0.0f32; uv.len()];
    for index in 0..rim_len {
        let rim_height = (rim_positions[index] - surface.centroid).dot(surface.normal);
        residual[index] = rim_height - surface.height(uv[index]);
    }
    harmonic_interior(&mut residual, rim_len, &triangles);

    // Lift the final planar cap onto the fitted surface plus blended residual.
    let generated = attrs
        .into_iter()
        .zip(uv.iter().zip(&residual))
        .skip(rim_len)
        .map(|(mut vertex, (&ab, &extra))| {
            vertex.position = surface.lift_with(ab, extra).to_array();
            vertex
        })
        .collect();
    RefinedCap {
        generated,
        triangles,
    }
}

/// The growable cap state shared by the refinement passes: planar positions,
/// per-vertex target scales, vertex attributes, and the triangulation.
struct CapPatch {
    uv: Vec<Vec2>,
    scale: Vec<f32>,
    attrs: Vec<EditVertex>,
    triangles: Vec<[usize; 3]>,
}

/// One bisection pass: every interior edge longer than `ALPHA` times its
/// local target scale is split at its midpoint with a conforming 2:4 rewrite
/// of both owner triangles. Returns whether anything split. Rim edges (one
/// owner) are never touched. Deterministic: edge keys are processed in sorted
/// order, and owners rewritten earlier in the pass fail the carries recheck.
fn bisect_pass(
    patch: &mut CapPatch,
    rim_len: usize,
    lift: impl Fn(Vec2) -> Vec3,
    policy: GeneratedVertexPolicy,
) -> bool {
    let CapPatch {
        uv,
        scale,
        attrs,
        triangles,
    } = patch;
    let mut split_any = false;
    let mut owners: std::collections::HashMap<(usize, usize), Vec<usize>> =
        std::collections::HashMap::new();
    for (triangle_index, &[a, b, c]) in triangles.iter().enumerate() {
        for (u, v) in [(a, b), (b, c), (c, a)] {
            owners
                .entry((u.min(v), u.max(v)))
                .or_default()
                .push(triangle_index);
        }
    }
    let mut keys: Vec<(usize, usize)> = owners.keys().copied().collect();
    keys.sort_unstable();
    for key in keys {
        if uv.len() - rim_len >= rim_len * MAX_GENERATED_PER_RIM {
            break;
        }
        let (u, v) = key;
        let Some(&[t1, t2]) = owners.get(&key).map(Vec::as_slice) else {
            continue; // Rim edge (one owner) or non-manifold noise.
        };
        // An earlier bisection this pass may have rewritten either owner.
        let carries = |triangle: [usize; 3]| triangle.contains(&u) && triangle.contains(&v);
        if !carries(triangles[t1]) || !carries(triangles[t2]) {
            continue;
        }
        let target = (scale[u] + scale[v]) * 0.5;
        if uv[u].distance(uv[v]) <= ALPHA * target {
            continue;
        }
        let midpoint_index = uv.len();
        let midpoint = (uv[u] + uv[v]) * 0.5;
        let vertex = midpoint_vertex(lift(midpoint), attrs, [u, v], policy);
        attrs.push(vertex);
        uv.push(midpoint);
        scale.push(target);
        // Conforming 2:4 split, winding preserved on all four children.
        for triangle_slot in [t1, t2] {
            let parent = triangles[triangle_slot];
            let mut keeps_u = parent;
            for slot in &mut keeps_u {
                if *slot == v {
                    *slot = midpoint_index;
                }
            }
            let mut keeps_v = parent;
            for slot in &mut keeps_v {
                if *slot == u {
                    *slot = midpoint_index;
                }
            }
            triangles[triangle_slot] = keeps_u;
            triangles.push(keeps_v);
        }
        split_any = true;
    }
    split_any
}

/// Raise the target edge scale so the estimated interior vertex count stays
/// within [`CAP_INTERIOR_BUDGET`]. Refinement density is quadratic in the rim
/// length for round holes; without this, a 20 000-edge rim generated 640 000
/// interior vertices and the fill ran for minutes. Rims small enough to fit
/// the budget (~250 edges for a round hole) are left byte-for-byte unchanged.
fn rescale_for_budget(uv: &[Vec2], rim_len: usize, scale: &mut [f32]) {
    if rim_len < 3 {
        return;
    }
    // Shoelace area of the projected rim polygon, in f64 for stable summation.
    let mut doubled_area = 0.0_f64;
    for index in 0..rim_len {
        let a = uv[index];
        let b = uv[(index + 1) % rim_len];
        doubled_area += f64::from(a.x) * f64::from(b.y) - f64::from(b.x) * f64::from(a.y);
    }
    let area = (doubled_area * 0.5).abs();
    let mut mean_scale = 0.0_f64;
    for &s in scale.iter().take(rim_len) {
        mean_scale += f64::from(s);
    }
    mean_scale /= f64::from(count_as_f32(rim_len));
    // Equilateral-triangle area at the target edge scale.
    let per_triangle = 3.0_f64.sqrt() / 4.0 * mean_scale * mean_scale;
    if !(per_triangle.is_finite() && per_triangle > 0.0) {
        return;
    }
    // Interior vertices approach half the triangle count for a dense patch;
    // bisection stops at edges up to ALPHA times the target scale, which
    // doubles the realized density versus the equilateral estimate (ALPHA^2),
    // so the two 2x factors cancel: estimate = area / per_triangle * 0.5 * 2.
    let estimated = area / per_triangle;
    let budget = f64::from(u32::try_from(CAP_INTERIOR_BUDGET).unwrap_or(u32::MAX));
    if estimated <= budget {
        return;
    }
    let factor = (estimated / budget).sqrt();
    if !factor.is_finite() {
        return;
    }
    // f64 -> f32: factor is in (1, sqrt(area/budget)]; well within f32 range.
    #[allow(clippy::cast_possible_truncation)]
    let factor = factor.min(f64::from(f32::MAX)) as f32;
    for s in scale.iter_mut() {
        *s *= factor;
    }
}

/// Harmonically interpolate a scalar field over the cap interior with the rim
/// values held fixed (Laplace with Dirichlet boundary): each interior value
/// converges to the mean of its neighbors. Used to blend the rim residual
/// inward so the cap meets the surrounding surface without a slope crease.
fn harmonic_interior(values: &mut [f32], rim_len: usize, triangles: &[[usize; 3]]) {
    if values.len() <= rim_len {
        return;
    }
    let mut neighbors: Vec<Vec<usize>> = vec![Vec::new(); values.len()];
    for &[a, b, c] in triangles {
        for (u, v) in [(a, b), (b, c), (c, a)] {
            if !neighbors[u].contains(&v) {
                neighbors[u].push(v);
            }
            if !neighbors[v].contains(&u) {
                neighbors[v].push(u);
            }
        }
    }
    // Convergence tolerance relative to the boundary data magnitude: once a
    // sweep moves nothing beyond it, further sweeps are numeric noise.
    let mut max_abs = 0.0_f32;
    for &value in values.iter().take(rim_len) {
        max_abs = max_abs.max(value.abs());
    }
    let tolerance = max_abs * 1e-4;
    for _ in 0..HARMONIC_ITERATIONS {
        let mut max_delta = 0.0_f32;
        for index in rim_len..values.len() {
            let ring = &neighbors[index];
            if ring.is_empty() {
                continue;
            }
            let mut sum = 0.0;
            for &neighbor in ring {
                sum += values[neighbor];
            }
            let updated = sum / count_as_f32(ring.len());
            max_delta = max_delta.max((updated - values[index]).abs());
            values[index] = updated;
        }
        if max_delta <= tolerance {
            break;
        }
    }
}

/// Fit a local frame (Newell normal over the rim) plus a quadric height field
/// least-squares fitted to the rim AND a band of surface samples just outside
/// it. A clean circular rim is nearly planar and carries no curvature on its
/// own, so the outside band is what lets the fit recover the local shape (a
/// sphere/saddle exactly, a gentle blend otherwise).
fn fit_cap_surface(rim: &[Vec3], support: &[[f32; 3]]) -> CapSurface {
    let rim_len = rim.len();
    let mut centroid = Vec3::ZERO;
    for &p in rim {
        centroid += p;
    }
    centroid /= count_as_f32(rim_len.max(1));

    // Newell's method: robust polygon normal for a non-planar rim. Vertices
    // are taken RELATIVE to the centroid: Newell is translation-invariant in
    // exact arithmetic, and centering avoids the catastrophic f32 cancellation
    // a small far-from-origin rim would otherwise hit.
    let mut normal = Vec3::ZERO;
    for index in 0..rim_len {
        let current = rim[index] - centroid;
        let next = rim[(index + 1) % rim_len] - centroid;
        normal.x += (current.y - next.y) * (current.z + next.z);
        normal.y += (current.z - next.z) * (current.x + next.x);
        normal.z += (current.x - next.x) * (current.y + next.y);
    }
    let normal = if normal.is_finite() && normal.length_squared() > f32::EPSILON {
        normal.normalize()
    } else {
        Vec3::Z
    };
    let (tangent_u, tangent_v) = basis_from_normal(normal);

    // Least-squares quadric h = c0 + c1 a + c2 b + c3 a^2 + c4 a b + c5 b^2,
    // solving the 6x6 normal equations (A^T A + ridge) c = A^T h. The rim pins
    // the fit at the seam; the outside support band supplies the curvature.
    //
    // The fit runs in SCALE-NORMALIZED coordinates (divided by the RMS planar
    // radius): the fixed ridge is then meaningful for every hole size, where
    // in raw mm a sub-millimeter hole was flattened (ridge dominated its tiny
    // quadratic terms) and a very large one was effectively unregularized.
    let planar: Vec<(f32, f32, f32)> = rim
        .iter()
        .copied()
        .chain(support.iter().map(|point| Vec3::from_array(*point)))
        .map(|sample| {
            let relative = sample - centroid;
            (
                relative.dot(tangent_u),
                relative.dot(tangent_v),
                relative.dot(normal),
            )
        })
        .collect();
    let mut radius_sq_sum = 0.0_f64;
    for &(coord_u, coord_v, _) in &planar {
        radius_sq_sum +=
            f64::from(coord_u) * f64::from(coord_u) + f64::from(coord_v) * f64::from(coord_v);
    }
    let sample_count = count_as_f32(planar.len().max(1));
    // f64 -> f32: an RMS of finite f32 radii; well within f32 range.
    #[allow(clippy::cast_possible_truncation)]
    let rms_radius = ((radius_sq_sum / f64::from(sample_count)).sqrt()) as f32;
    if !(rms_radius.is_finite() && rms_radius > f32::EPSILON) {
        return CapSurface {
            centroid,
            u: tangent_u,
            v: tangent_v,
            normal,
            coeffs: [0.0; 6],
        };
    }
    let inv_radius = 1.0 / rms_radius;

    let mut normal_matrix = [[0.0f32; 6]; 6];
    let mut normal_rhs = [0.0f32; 6];
    for &(coord_u, coord_v, height) in &planar {
        let (coord_u, coord_v, height) = (
            coord_u * inv_radius,
            coord_v * inv_radius,
            height * inv_radius,
        );
        let basis = [
            1.0,
            coord_u,
            coord_v,
            coord_u * coord_u,
            coord_u * coord_v,
            coord_v * coord_v,
        ];
        for (i, &bi) in basis.iter().enumerate() {
            for (j, &bj) in basis.iter().enumerate() {
                normal_matrix[i][j] += bi * bj;
            }
            normal_rhs[i] += bi * height;
        }
    }
    for (i, row) in normal_matrix.iter_mut().enumerate() {
        row[i] += QUADRIC_RIDGE;
    }
    let scaled = solve6(normal_matrix, normal_rhs).unwrap_or([0.0; 6]);
    // Undo the normalization: h = s*c0' + c1'*a + c2'*b + (c3'/s)*a^2 + ...
    let coeffs = [
        scaled[0] * rms_radius,
        scaled[1],
        scaled[2],
        scaled[3] * inv_radius,
        scaled[4] * inv_radius,
        scaled[5] * inv_radius,
    ];

    CapSurface {
        centroid,
        u: tangent_u,
        v: tangent_v,
        normal,
        coeffs,
    }
}

/// Lossless-enough count-to-float for averaging small vertex fans. Cap sizes
/// never approach `u16::MAX`, so the saturation only guards a pathological rim.
fn count_as_f32(count: usize) -> f32 {
    f32::from(u16::try_from(count).unwrap_or(u16::MAX))
}

/// Right-handed orthonormal tangent basis for a unit `normal`.
fn basis_from_normal(normal: Vec3) -> (Vec3, Vec3) {
    let axis = if normal.x.abs() > 0.9 {
        Vec3::Y
    } else {
        Vec3::X
    };
    let u = axis.cross(normal).normalize();
    let v = normal.cross(u);
    (u, v)
}

/// Solve a 6x6 linear system by Gaussian elimination with partial pivoting.
/// Returns `None` if the matrix is singular (caller falls back to a plane).
fn solve6(mut m: [[f32; 6]; 6], mut b: [f32; 6]) -> Option<[f32; 6]> {
    for col in 0..6 {
        // Partial pivot.
        let mut pivot = col;
        for row in (col + 1)..6 {
            if m[row][col].abs() > m[pivot][col].abs() {
                pivot = row;
            }
        }
        if m[pivot][col].abs() < 1e-12 {
            return None;
        }
        m.swap(col, pivot);
        b.swap(col, pivot);
        let inv = 1.0 / m[col][col];
        for row in (col + 1)..6 {
            let factor = m[row][col] * inv;
            if factor == 0.0 {
                continue;
            }
            for k in col..6 {
                m[row][k] -= factor * m[col][k];
            }
            b[row] -= factor * b[col];
        }
    }
    let mut x = [0.0f32; 6];
    for row in (0..6).rev() {
        let mut sum = b[row];
        for (col, &solved) in x.iter().enumerate().skip(row + 1) {
            sum -= m[row][col] * solved;
        }
        x[row] = sum / m[row][row];
    }
    Some(x)
}

/// Attributes for a bisection midpoint: the average of its edge endpoints
/// (or the deterministic neutral fallback).
fn midpoint_vertex(
    position: Vec3,
    attrs: &[EditVertex],
    endpoints: [usize; 2],
    policy: GeneratedVertexPolicy,
) -> EditVertex {
    match policy {
        GeneratedVertexPolicy::NeutralFallback => EditVertex::at(position.to_array()),
        GeneratedVertexPolicy::InterpolateBoundary => {
            let mut color = [0u16; 4];
            let mut uv = [0.0f32; 2];
            for &endpoint in &endpoints {
                let vertex = &attrs[endpoint];
                for (sum, &channel) in color.iter_mut().zip(&vertex.color) {
                    *sum += u16::from(channel);
                }
                uv[0] += vertex.uv[0];
                uv[1] += vertex.uv[1];
            }
            let mut vertex = EditVertex::at(position.to_array());
            for (channel, &sum) in vertex.color.iter_mut().zip(&color) {
                // Average of two u8 channels always fits back into u8.
                *channel = u8::try_from(sum / 2).unwrap_or(u8::MAX);
            }
            vertex.uv = [uv[0] / 2.0, uv[1] / 2.0];
            vertex
        }
    }
}
