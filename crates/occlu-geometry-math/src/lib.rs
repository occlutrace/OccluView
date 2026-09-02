//! Shared geometry constants and trivial pure math for OccluView kernels.
//!
//! This is the bottom layer of the workspace: it imports nothing but glam, and
//! higher-level OccluView crates may depend on it without creating cycles. The
//! constants here decide which vertices weld and which facets shade; they used
//! to be copied across `occlu-mesh-edit`, `occluview-core` and
//! `occluview-hps`, and the copies
//! drifted once (the 2026-07-25 fix took four weeks to reach all three
//! crates). One home means one change.

#![forbid(unsafe_code)]

use glam::Vec3;

/// Squared sine of the smallest angle a facet may have and still contribute a
/// normal. Scale-free: the test compares twice the facet's area against its own
/// longest edge squared, so it means the same thing on a 10 mm arch and on a
/// 10 um sliver.
///
/// A facet is degenerate when its area falls below this fraction of its own
/// longest edge squared. `DEGENERATE_SIN` in `occlu-mesh-edit`'s repair module
/// is a different threshold (a relative-sine test, `f64 = 1e-5`) and must not
/// be merged with this one.
pub const DEGENERATE_AREA_SIN: f32 = 1e-10;

/// A facet is degenerate when its area falls below this fraction of its own
/// longest edge squared.
#[inline]
#[must_use]
pub fn facet_contributes_normal(longest_edge_sq: f32, face_normal_length_sq: f32) -> bool {
    face_normal_length_sq > longest_edge_sq * longest_edge_sq * DEGENERATE_AREA_SIN
}

/// Above this many vertices sharing one position, normal agreement is judged
/// against the group mean rather than pairwise.
///
/// Well past any real vertex valence -- a fan around one point is tens of
/// triangles, not hundreds -- so no scan reaches it, and a crafted file cannot
/// spend minutes here. Used by both the loader (core) and the edit kernels
/// (mesh-edit), so the two agree about how much work one pile of coincident
/// vertices is worth.
pub const MAX_PAIRWISE_DUPLICATE_GROUP: usize = 256;

/// How many directions one coincident group may hold before it is left alone.
///
/// A pile that genuinely points sixteen ways is not a surface any averaging can
/// help, and the pass costs one dot product per member per cluster.
pub const MAX_DUPLICATE_CLUSTERS: usize = 16;

/// Dot-product threshold for two normals to count as the same direction when
/// averaging a coincident-position group. One name across crates; it used to
/// be `DUPLICATE_NORMAL_DOT` in `occlu-mesh-edit` and
/// `SMOOTH_DUPLICATE_NORMAL_DOT` in `occluview-core`.
pub const DUPLICATE_NORMAL_DOT: f32 = 0.5;

/// Two positions within this distance are the same point for shading.
///
/// One number decides which vertices share a normal, and three crates need it:
/// core welds at load, mesh-edit welds after every brush stroke and hole fill,
/// and the same scan must shade the same way on both paths. Written twice,
/// under two names, with two byte-identical key functions, the same scan
/// shades one way on open and another way after any edit -- a seam that
/// appears mid-session with nothing to blame.
pub const COINCIDENT_POSITION_EPS_MM: f32 = 0.002;

/// Quantize a position onto the [`COINCIDENT_POSITION_EPS_MM`] lattice.
///
/// Equal keys mean "the same point" for normal welding. Shared by the loader
/// and the edit kernels so both sides of an edit agree.
#[must_use]
pub fn coincident_position_key(position: [f32; 3]) -> [i32; 3] {
    [
        position_lane_key(position[0]),
        position_lane_key(position[1]),
        position_lane_key(position[2]),
    ]
}

#[allow(clippy::cast_possible_truncation)]
fn position_lane_key(value: f32) -> i32 {
    if !value.is_finite() {
        return 0;
    }

    let scaled = f64::from(value / COINCIDENT_POSITION_EPS_MM).round();
    if scaled <= f64::from(i32::MIN) {
        i32::MIN
    } else if scaled >= f64::from(i32::MAX) {
        i32::MAX
    } else {
        scaled as i32
    }
}

/// Area-weighted vertex normals, from a triangle list and a position lookup.
///
/// The lookup is a closure rather than a `&[Vec3]` so a caller holding
/// interleaved vertices does not have to copy every position out first -- on a
/// six-million-vertex scan that copy would be seventy megabytes to avoid one
/// duplicated loop.
///
/// A triangle with an out-of-range corner is skipped rather than trusted; a
/// degenerate facet (below [`DEGENERATE_AREA_SIN`] of its own longest edge
/// squared) contributes nothing, so a vertex that only ever touches degenerate
/// facets keeps the zero normal the caller's fallback fills in.
#[must_use]
pub fn accumulate_smooth_normals(
    vertex_count: usize,
    indices: &[u32],
    position: impl Fn(usize) -> Option<Vec3>,
) -> Vec<Vec3> {
    let mut normals = vec![Vec3::ZERO; vertex_count];
    for triangle in indices.as_chunks::<3>().0 {
        let ia = triangle[0] as usize;
        let ib = triangle[1] as usize;
        let ic = triangle[2] as usize;
        let (Some(a), Some(b), Some(c)) = (position(ia), position(ib), position(ic)) else {
            continue;
        };
        let face_normal = (b - a).cross(c - a);
        let longest_edge_sq = (b - a)
            .length_squared()
            .max((c - b).length_squared())
            .max((a - c).length_squared());
        if face_normal.is_finite()
            && facet_contributes_normal(longest_edge_sq, face_normal.length_squared())
        {
            normals[ia] += face_normal;
            normals[ib] += face_normal;
            normals[ic] += face_normal;
        }
    }
    normals
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_tolerance_decides_which_vertices_share_a_normal() {
        // The seam described on `COINCIDENT_POSITION_EPS_MM`: two copies of
        // the number and the same scan shades one way on open, another after
        // any edit.
        let origin = [0.0_f32, 0.0, 0.0];
        let inside = [COINCIDENT_POSITION_EPS_MM * 0.4, 0.0, 0.0];
        let outside = [COINCIDENT_POSITION_EPS_MM * 4.0, 0.0, 0.0];

        assert_eq!(
            coincident_position_key(origin),
            coincident_position_key(inside),
            "positions inside the tolerance must be one point"
        );
        assert_ne!(
            coincident_position_key(origin),
            coincident_position_key(outside),
            "positions well outside it must not be"
        );
        // Non-finite input has to answer something rather than panic: it
        // arrives from files.
        assert_eq!(
            coincident_position_key([f32::NAN, f32::INFINITY, 0.0]),
            [0, 0, 0]
        );
    }

    #[test]
    fn degenerate_gate_matches_the_accumulator() {
        // A right triangle with unit legs has cross-product length 1.0 (twice
        // its area) and longest edge squared 2, so it is healthy: the gate is
        // `len_sq > longest_edge_sq^2 * DEGENERATE_AREA_SIN`. A sliver 1e-6
        // wide has cross length ~1e-6 and the same longest edge, so it is
        // degenerate.
        let right: [f32; 3] = [0.0, 0.0, 0.0];
        let sliver: [f32; 3] = [1e-6, 0.0, 0.0];
        let third: [f32; 3] = [0.0, 1.0, 0.0];
        let positions = [right, sliver, third];
        let normals =
            accumulate_smooth_normals(3, &[0, 1, 2], |i| Some(Vec3::from_array(positions[i])));
        assert_eq!(
            normals[0],
            Vec3::ZERO,
            "a facet below the gate must not contribute a normal"
        );
        assert!(
            facet_contributes_normal(2.0, 1.0),
            "a healthy facet must contribute"
        );
        assert!(
            !facet_contributes_normal(2.0, 1e-12),
            "a sliver far below the gate must not contribute"
        );
    }

    #[test]
    fn out_of_range_triangle_is_skipped() {
        let normals = accumulate_smooth_normals(3, &[0, 1, 9], |i| match i {
            0 => Some(Vec3::X),
            1 => Some(Vec3::Y),
            _ => None,
        });
        assert_eq!(normals, vec![Vec3::ZERO; 3]);
    }
}
