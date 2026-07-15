//! STL-soup Close Holes coverage.
//!
//! An STL loads as a triangle SOUP: every triangle stores its three corners as
//! fresh, independent vertices (see `occluview-formats` binary STL reader), so
//! no two triangles share a vertex index. In index space EVERY edge is then a
//! boundary half-edge and the whole model reads as a cloud of disconnected
//! needles. On such input the pre-fill cut-line healing used to see every
//! triangle as an "isolated nick" and chew the entire mesh — the owner's
//! "524560 nicks healed, none closed" on a real digital-waxup STL.
//!
//! These fixtures reproduce that exact shape: build a CLOSED welded solid,
//! explode it to soup (the STL round-trip), punch real holes, and assert Close
//! Holes now reports honest counts and actually closes the rims — including
//! strongly curved ones.

// Grid/geometry fixtures use conventional short axis names (i, j, u, v, x, y).
#![allow(clippy::many_single_char_names)]

use crate::holes::fill_holes;
use crate::topology::weld_soup_topology;
use crate::{EditVertex, MeshEditBuffers, MeshEditOptions, MeshTopology};
use glam::Vec3;
use std::collections::HashSet;

/// A closed, watertight, everywhere-curved torus as a welded indexed mesh.
/// Wraps in both parameters, so it has no boundary at all — every genuine
/// boundary in a punched fixture is a hole we made, never a fixture artifact.
fn torus(nu: usize, nv: usize, big_r: f32, small_r: f32) -> MeshEditBuffers {
    let mut vertices = Vec::with_capacity(nu * nv);
    for j in 0..nv {
        for i in 0..nu {
            let u = i as f32 / nu as f32 * std::f32::consts::TAU;
            let v = j as f32 / nv as f32 * std::f32::consts::TAU;
            let ring = big_r + small_r * v.cos();
            vertices.push(EditVertex::at([
                ring * u.cos(),
                ring * u.sin(),
                small_r * v.sin(),
            ]));
        }
    }
    let idx = |i: usize, j: usize| ((j % nv) * nu + (i % nu)) as u32;
    let mut indices = Vec::with_capacity(nu * nv * 6);
    for j in 0..nv {
        for i in 0..nu {
            let (a, b, c, d) = (idx(i, j), idx(i + 1, j), idx(i + 1, j + 1), idx(i, j + 1));
            indices.extend_from_slice(&[a, b, c, a, c, d]);
        }
    }
    MeshEditBuffers {
        vertices,
        indices,
        topology: MeshTopology::TriangleMesh,
    }
}

/// Explode a welded mesh into an STL-style soup: one fresh vertex per triangle
/// corner, sequential indices, no sharing. This is byte-for-byte the topology a
/// binary STL reader produces.
fn explode_to_soup(mesh: &MeshEditBuffers) -> MeshEditBuffers {
    let mut vertices = Vec::with_capacity(mesh.indices.len());
    let mut indices = Vec::with_capacity(mesh.indices.len());
    for &vi in &mesh.indices {
        indices.push(vertices.len() as u32);
        vertices.push(mesh.vertices[vi as usize]);
    }
    MeshEditBuffers {
        vertices,
        indices,
        topology: MeshTopology::TriangleMesh,
    }
}

/// Remove triangles whose centroid lands inside any hole ball. Operates on soup
/// indices, leaving the surrounding rim as a genuine (curved) boundary loop.
fn punch_holes(mesh: &MeshEditBuffers, holes: &[(Vec3, f32)]) -> MeshEditBuffers {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    for tri in mesh.indices.chunks_exact(3) {
        let c = tri
            .iter()
            .map(|&i| Vec3::from_array(mesh.vertices[i as usize].position))
            .sum::<Vec3>()
            / 3.0;
        if holes.iter().any(|&(center, r)| (c - center).length() <= r) {
            continue;
        }
        for &i in tri {
            indices.push(vertices.len() as u32);
            vertices.push(mesh.vertices[i as usize]);
        }
    }
    MeshEditBuffers {
        vertices,
        indices,
        topology: MeshTopology::TriangleMesh,
    }
}

/// Count boundary half-edges (directed edges with no opposing twin). Zero means
/// watertight.
fn open_edge_count(mesh: &MeshEditBuffers) -> usize {
    let mut directed: HashSet<(u32, u32)> = HashSet::with_capacity(mesh.indices.len());
    for tri in mesh.indices.chunks_exact(3) {
        for e in [(tri[0], tri[1]), (tri[1], tri[2]), (tri[2], tri[0])] {
            directed.insert(e);
        }
    }
    directed
        .iter()
        .filter(|&&(a, b)| !directed.contains(&(b, a)))
        .count()
}

/// The production Close Holes button options (whole-mesh, healed cut line).
fn close_holes_options() -> MeshEditOptions {
    MeshEditOptions {
        compact_vertices: true,
        heal_boundary_rims: true,
        ..MeshEditOptions::default()
    }
}

/// A point on the torus surface at parameters (u, v) in turns, nudged outward so
/// the centroid test around it removes a clean cap.
fn torus_point(big_r: f32, small_r: f32, u_turn: f32, v_turn: f32) -> Vec3 {
    let (u, v) = (
        u_turn * std::f32::consts::TAU,
        v_turn * std::f32::consts::TAU,
    );
    let ring = big_r + small_r * v.cos();
    Vec3::new(ring * u.cos(), ring * u.sin(), small_r * v.sin())
}

/// THE bug: a soup STL with punched holes must report honest counts (no phantom
/// "nicks", no chewed mesh) and actually close every curved rim watertight.
#[test]
fn soup_close_holes_reports_honest_counts_and_closes_curved_rims() {
    let (big_r, small_r) = (10.0_f32, 3.0_f32);
    let welded = torus(96, 48, big_r, small_r);
    let soup = explode_to_soup(&welded);
    // Three holes on strongly curved parts of the tube (inner wall, top, outer).
    let holes = [
        (torus_point(big_r, small_r, 0.10, 0.50), 1.6_f32), // inner wall
        (torus_point(big_r, small_r, 0.55, 0.25), 1.6),     // top shoulder
        (torus_point(big_r, small_r, 0.80, 0.00), 1.6),     // outer equator
    ];
    let punched = punch_holes(&soup, &holes);
    assert!(
        open_edge_count(&punched) > 0,
        "punching must open real boundary loops"
    );
    let input_triangles = punched.triangle_count();

    let closed = fill_holes(&punched, None, close_holes_options()).expect("close holes");

    // Honest counts: the soup's half-million phantom needles are NOT reported as
    // healed nicks. A clean punched solid has no genuine cut-line damage.
    assert_eq!(
        closed.report.healed_rims, 0,
        "soup duplicates must never be counted as healed nicks"
    );
    assert_eq!(
        closed.report.skipped_damaged_rims, 0,
        "honest curved rims must not be refused as damaged"
    );
    // The real holes close.
    assert!(
        closed.report.filled_holes >= holes.len(),
        "every punched hole must close (got {})",
        closed.report.filled_holes
    );
    // The mesh is not chewed away: the body survives and gains caps.
    assert!(
        closed.report.output_triangles >= input_triangles,
        "the body must survive and gain caps, not be deleted (out {}, in {})",
        closed.report.output_triangles,
        input_triangles
    );
    // Watertight result.
    assert_eq!(
        open_edge_count(&closed.mesh),
        0,
        "the closed soup solid must be watertight"
    );
}

/// The soup weld recovers the exact shared topology and is idempotent: welding
/// an already-welded mesh is a no-op (`None`), so there is no double-weld when
/// the repair pipeline — which welds itself — happens to hand welded buffers to
/// the shared hole machinery.
#[test]
fn soup_weld_recovers_topology_and_is_idempotent() {
    let welded = torus(40, 20, 10.0, 3.0);
    let soup = explode_to_soup(&welded);
    // Soup: 3 corners per triangle, nothing shared.
    assert_eq!(soup.vertices.len(), soup.indices.len());

    let recovered = weld_soup_topology(&soup)
        .expect("weld ok")
        .expect("soup must weld");
    // Triangle order and count are preserved (selection stays valid).
    assert_eq!(recovered.indices.len(), soup.indices.len());
    // The distinct referenced vertex ids collapse back to the welded count.
    let distinct: HashSet<u32> = recovered.indices.iter().copied().collect();
    assert_eq!(
        distinct.len(),
        welded.vertices.len(),
        "welding soup recovers exactly the shared vertices"
    );
    // The recovered mesh is closed again (torus has no boundary).
    assert_eq!(open_edge_count(&recovered), 0, "the torus re-closes");

    // Idempotent: an already-welded mesh does not weld again.
    assert!(
        weld_soup_topology(&welded).expect("weld ok").is_none(),
        "a welded mesh must pass through unchanged (no double-weld)"
    );
    // ...and welding the recovered soup a second time is also a no-op, because
    // its referenced vertices are now the unique representatives.
    let recompacted = crate::delete_crop::crop_to_selected_faces(
        &recovered,
        &crate::FaceSelection::new(vec![true; recovered.triangle_count()]),
        MeshEditOptions {
            compact_vertices: true,
            ..MeshEditOptions::default()
        },
    )
    .expect("compact")
    .mesh;
    assert!(
        weld_soup_topology(&recompacted).expect("weld ok").is_none(),
        "a compacted welded mesh must not weld again"
    );
}

/// Close Holes on soup is deterministic: the same input yields a byte-identical
/// mesh and report twice.
#[test]
fn soup_close_holes_is_deterministic() {
    let (big_r, small_r) = (10.0_f32, 3.0_f32);
    let soup = explode_to_soup(&torus(72, 36, big_r, small_r));
    let holes = [
        (torus_point(big_r, small_r, 0.20, 0.50), 1.7_f32),
        (torus_point(big_r, small_r, 0.65, 0.10), 1.7),
    ];
    let punched = punch_holes(&soup, &holes);
    let a = fill_holes(&punched, None, close_holes_options()).expect("close a");
    let b = fill_holes(&punched, None, close_holes_options()).expect("close b");
    assert_eq!(a.mesh.vertices, b.mesh.vertices);
    assert_eq!(a.mesh.indices, b.mesh.indices);
    assert_eq!(a.report, b.report);
}

/// The mm-perimeter path (the app's Close Holes slider) also welds the soup
/// first, so small curved holes close honestly under a budget.
#[test]
fn soup_close_holes_with_mm_limit_closes_small_curved_holes() {
    let (big_r, small_r) = (10.0_f32, 3.0_f32);
    let soup = explode_to_soup(&torus(96, 48, big_r, small_r));
    let holes = [(torus_point(big_r, small_r, 0.33, 0.5), 1.5_f32)];
    let punched = punch_holes(&soup, &holes);

    let options = MeshEditOptions {
        compact_vertices: true,
        heal_boundary_rims: true,
        max_rim_perimeter_mm: Some(30.0),
        ..MeshEditOptions::default()
    };
    let closed = fill_holes(&punched, None, options).expect("close");
    assert_eq!(
        closed.report.healed_rims, 0,
        "no phantom nicks under mm path"
    );
    assert!(closed.report.filled_holes >= 1, "the small hole closes");
    assert_eq!(open_edge_count(&closed.mesh), 0, "watertight");
}

/// Faces whose centroid lies within `radius` of `center` — the operator
/// lassoing the socket region around a hole.
fn select_faces_near(mesh: &MeshEditBuffers, center: Vec3, radius: f32) -> Vec<bool> {
    mesh.indices
        .chunks_exact(3)
        .map(|tri| {
            let c = tri
                .iter()
                .map(|&i| Vec3::from_array(mesh.vertices[i as usize].position))
                .sum::<Vec3>()
                / 3.0;
            (c - center).length() <= radius
        })
        .collect()
}

/// A LARGE hole punched across the tightest curvature of the tube (the inner
/// wall, where the surface bends most), closed via a lasso selection — the
/// "загибы" (curved rims) the owner said the tool refused. With the operator's
/// explicit selection the border guard is off, so the honest strongly-curved
/// rim must close through the fallback chain (interpolated → membrane → lid) and
/// is never refused as damaged.
#[test]
fn soup_strongly_curved_inner_wall_socket_closes_with_selection() {
    let (big_r, small_r) = (8.0_f32, 2.5_f32);
    let soup = explode_to_soup(&torus(160, 80, big_r, small_r));
    // A wide cap straddling the inner wall (v ~ 0.5 turn) wraps around the
    // tube's high-curvature crease — larger than half the tube radius.
    let center = torus_point(big_r, small_r, 0.30, 0.5);
    let punched = punch_holes(&soup, &[(center, 2.2_f32)]);
    assert!(open_edge_count(&punched) > 0, "the curved rim is open");

    // Lasso the socket: every face within a ring around the hole.
    let selection = crate::FaceSelection::new(select_faces_near(&punched, center, 3.4));
    let closed = fill_holes(&punched, Some(&selection), close_holes_options()).expect("close");
    assert_eq!(
        closed.report.skipped_damaged_rims, 0,
        "an honest curved rim must not be refused as damaged"
    );
    assert!(
        closed.report.filled_holes >= 1,
        "the strongly curved socket must close (got {})",
        closed.report.filled_holes
    );
    assert_eq!(open_edge_count(&closed.mesh), 0, "watertight after close");
}

/// A COLOR SEAM (coincident positions, different vertex colors — flat-shaded
/// CAD exports do this) is an index-space slit. Close Holes fuses it during
/// rim healing ON PURPOSE: leaving the slit open would hand the filler two
/// giant phantom rims to wall off with membranes. The pinned contract: the
/// real hole closes, the seam fuses into closed topology (honest healed
/// counter), and NO phantom membranes are added along the seam line.
#[test]
fn color_seam_fuses_into_closed_topology_instead_of_growing_membranes() {
    let (nu, nv) = (48_usize, 24_usize);
    let (big_r, small_r) = (10.0_f32, 3.0_f32);
    let mut seamed = torus(nu, nv, big_r, small_r);
    // Split one meridian (i == 0) into a color seam: duplicate its vertices
    // with a red color and point the triangles on the i == nu-1 side at the
    // duplicates. Positions are bit-identical; colors differ.
    let base_count = seamed.vertices.len();
    let mut duplicate_of = vec![u32::MAX; base_count];
    for j in 0..nv {
        let original = (j * nu) as u32;
        let mut red = seamed.vertices[original as usize];
        red.color = [220, 40, 40, 255];
        duplicate_of[original as usize] = (seamed.vertices.len()) as u32;
        seamed.vertices.push(red);
    }
    // Triangles of the LAST column (i == nu-1) reference i == 0 vertices
    // through the wrap; re-point exactly those references at the duplicates.
    let triangle_count = seamed.indices.len() / 3;
    for t in 0..triangle_count {
        // Two triangles per quad, j-major: quad (i, j) owns triangles 2q, 2q+1.
        if (t / 2) % nu != nu - 1 {
            continue;
        }
        for k in 0..3 {
            let v = seamed.indices[t * 3 + k] as usize;
            let dup = duplicate_of[v];
            if dup != u32::MAX {
                seamed.indices[t * 3 + k] = dup;
            }
        }
    }
    assert!(
        open_edge_count(&seamed) > 0,
        "the color seam must read as an index-space slit"
    );
    // Punch one REAL hole away from the seam.
    let punched = punch_holes(
        &seamed,
        &[(torus_point(big_r, small_r, 0.55, 0.25), 1.6_f32)],
    );
    let input_triangles = punched.triangle_count();

    let closed = fill_holes(&punched, None, close_holes_options()).expect("close holes");

    assert!(
        closed.report.filled_holes >= 1,
        "the real hole must close (got {})",
        closed.report.filled_holes
    );
    assert_eq!(
        open_edge_count(&closed.mesh),
        0,
        "seam and hole both end closed — watertight output"
    );
    // No phantom membranes: the only added geometry is the hole cap. A walled
    // seam would add on the order of 2*nv extra triangles along the meridian.
    let added = closed
        .report
        .output_triangles
        .saturating_sub(input_triangles);
    assert!(
        added < nv,
        "seam must fuse, not grow membrane walls (added {added} triangles)"
    );
}
