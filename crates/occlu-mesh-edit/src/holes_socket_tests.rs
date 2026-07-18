//! The tooth-socket close-holes workflow: digitally extract a tooth (lasso
//! delete), then close the socket. Regression coverage for the "closed N holes
//! (M damaged rims skipped)" bug — a jagged cut left needle nicks and a socket
//! rim too big for the old min-area cap, so the one rim the operator wanted
//! closed stayed open. These tests exercise the REAL kernels the app runs.

// Grid/geometry fixtures use conventional short axis names (i, j, u, v, x, y).
#![allow(clippy::many_single_char_names)]

use crate::cap_minweight::{min_area_triangulation_any, rim_is_simple_3d};
use crate::delete_crop::delete_selected_faces;
use crate::holes::fill_holes;
use crate::holes_cleanup::heal_boundary_rims;
use crate::{EditVertex, FaceSelection, MeshEditBuffers, MeshEditOptions, MeshTopology};
use glam::Vec3;
use std::collections::{HashMap, HashSet};

/// A dense curved arch segment ("gum") carrying a raised tooth dome, so a
/// lasso deletion leaves a socket rim that wraps out of plane — the case the
/// planar caps refuse.
fn dome_with_tooth(nu: usize, nv: usize) -> MeshEditBuffers {
    let (width, depth, vault) = (20.0_f32, 14.0_f32, 6.0_f32);
    let (tooth_cx, tooth_r, tooth_h) = (0.15_f32, 0.26_f32, 2.4_f32);
    let mut vertices = Vec::with_capacity(nu * nv);
    for j in 0..nv {
        for i in 0..nu {
            let u = i as f32 / (nu - 1) as f32;
            let v = j as f32 / (nv - 1) as f32;
            let x = (u - 0.5) * width;
            let y = (v - 0.5) * depth;
            let mut z = -vault * (u - 0.5) * (u - 0.5) * 4.0;
            let du = u - 0.5 - tooth_cx;
            let dv = v - 0.5;
            z += tooth_h * (-(du * du + dv * dv) / (tooth_r * tooth_r)).exp();
            vertices.push(EditVertex::at([x, y, z]));
        }
    }
    let mut indices = Vec::with_capacity((nu - 1) * (nv - 1) * 6);
    let idx = |i: usize, j: usize| (j * nu + i) as u32;
    for j in 0..nv - 1 {
        for i in 0..nu - 1 {
            indices.extend_from_slice(&[idx(i, j), idx(i + 1, j), idx(i + 1, j + 1)]);
            indices.extend_from_slice(&[idx(i, j), idx(i + 1, j + 1), idx(i, j + 1)]);
        }
    }
    MeshEditBuffers {
        vertices,
        indices,
        topology: MeshTopology::TriangleMesh,
    }
}

/// A jagged "lasso" mask around the tooth: one connected region (a real tooth
/// is one region) whose high-frequency boundary makes the per-triangle centroid
/// test isolate needle nicks along the cut.
fn lasso_delete_mask(mesh: &MeshEditBuffers) -> Vec<bool> {
    let (cx, cy) = (0.15_f32 * 20.0, 0.0_f32);
    let segs = 220;
    let poly: Vec<[f32; 2]> = (0..segs)
        .map(|k| {
            let a = k as f32 / segs as f32 * std::f32::consts::TAU;
            let n = ((k as f32 * 12.9898).sin() * 43758.547).fract();
            let r = 3.2 * (0.90 + 0.14 * n);
            [cx + r * a.cos(), cy + r * a.sin()]
        })
        .collect();
    let inside = |p: [f32; 2]| -> bool {
        let mut hit = false;
        let mut j = poly.len() - 1;
        for i in 0..poly.len() {
            let (yi, yj) = (poly[i][1], poly[j][1]);
            if ((yi > p[1]) != (yj > p[1]))
                && (p[0] < (poly[j][0] - poly[i][0]) * (p[1] - yi) / (yj - yi) + poly[i][0])
            {
                hit = !hit;
            }
            j = i;
        }
        hit
    };
    mesh.indices
        .chunks_exact(3)
        .map(|tri| {
            let c = tri
                .iter()
                .map(|&i| Vec3::from_array(mesh.vertices[i as usize].position))
                .sum::<Vec3>()
                / 3.0;
            inside([c.x, c.y])
        })
        .collect()
}

/// Faces incident to any boundary vertex — the operator lassoing the socket.
fn rim_selection_mask(mesh: &MeshEditBuffers) -> Vec<bool> {
    let mut directed: HashSet<(u32, u32)> = HashSet::new();
    for tri in mesh.indices.chunks_exact(3) {
        for e in [(tri[0], tri[1]), (tri[1], tri[2]), (tri[2], tri[0])] {
            directed.insert(e);
        }
    }
    let mut boundary_vertex = vec![false; mesh.vertices.len()];
    for &(a, b) in &directed {
        if !directed.contains(&(b, a)) {
            boundary_vertex[a as usize] = true;
            boundary_vertex[b as usize] = true;
        }
    }
    mesh.indices
        .chunks_exact(3)
        .map(|tri| tri.iter().any(|&i| boundary_vertex[i as usize]))
        .collect()
}

/// Count boundary half-edges (directed edges without an opposing twin): zero
/// means the mesh is closed/watertight.
fn open_edge_count(mesh: &MeshEditBuffers) -> usize {
    let mut directed: HashSet<(u32, u32)> = HashSet::new();
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

/// The extracted-tooth workflow: delete then close. The socket must fully
/// close with ZERO damaged rims, the nick triangles healed away.
#[test]
fn tooth_socket_close_heals_nicks_and_closes_the_socket() {
    let mesh = dome_with_tooth(140, 80);
    let del = FaceSelection::new(lasso_delete_mask(&mesh));
    let deleted = delete_selected_faces(
        &mesh,
        &del,
        MeshEditOptions {
            compact_vertices: true,
            ..MeshEditOptions::default()
        },
    )
    .expect("delete");
    assert!(
        open_edge_count(&deleted.mesh) > 0,
        "the delete must open the socket"
    );

    let rim = FaceSelection::new(rim_selection_mask(&deleted.mesh));
    let closed = fill_holes(
        &deleted.mesh,
        Some(&rim),
        MeshEditOptions {
            compact_vertices: true,
            heal_boundary_rims: true,
            ..MeshEditOptions::default()
        },
    )
    .expect("close");

    assert_eq!(
        closed.report.skipped_damaged_rims, 0,
        "no rim may be refused as damaged after healing"
    );
    assert!(
        closed.report.healed_rims > 0,
        "the jagged cut leaves needle nicks that must be healed"
    );
    assert!(closed.report.filled_holes >= 1, "the socket must close");
    assert_eq!(
        open_edge_count(&closed.mesh),
        0,
        "the closed socket must be watertight"
    );
}

/// The socket rim is several hundred edges — past the old 256 min-area cap.
/// The hierarchical membrane must cap it, and the result must be a complete,
/// internally manifold fan.
#[test]
fn hierarchical_membrane_caps_a_large_rim_watertight() {
    // A wavy 3D ring of 400 points (out of plane, so the planar ear-clip would
    // struggle) — well past the 256 leaf size.
    let n = 400;
    let points: Vec<Vec3> = (0..n)
        .map(|k| {
            let a = k as f32 / n as f32 * std::f32::consts::TAU;
            let r = 10.0 + 0.4 * (a * 7.0).sin();
            Vec3::new(r * a.cos(), r * a.sin(), 1.5 * (a * 5.0).cos())
        })
        .collect();

    let tris = min_area_triangulation_any(&points).expect("large rim must triangulate");
    assert_eq!(tris.len(), n - 2, "a full fan has n - 2 triangles");

    // Internally manifold: interior edges in exactly two triangles, and exactly
    // the n rim edges left as boundary.
    let mut edge_use: HashMap<(usize, usize), i32> = HashMap::new();
    for t in &tris {
        assert!(t[0] < n && t[1] < n && t[2] < n);
        assert!(
            t[0] != t[1] && t[1] != t[2] && t[2] != t[0],
            "no degenerate"
        );
        for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            *edge_use.entry((a.min(b), a.max(b))).or_default() += 1;
        }
    }
    assert!(
        edge_use.values().all(|&c| c <= 2),
        "no interior edge may be shared by more than two cap triangles"
    );
    assert_eq!(
        edge_use.values().filter(|&&c| c == 1).count(),
        n,
        "exactly the rim edges stay on the boundary"
    );
}

/// The hierarchical split is deterministic: the same rim gives the same cap.
#[test]
fn hierarchical_membrane_is_deterministic() {
    let n = 512;
    let points: Vec<Vec3> = (0..n)
        .map(|k| {
            let a = k as f32 / n as f32 * std::f32::consts::TAU;
            Vec3::new(10.0 * a.cos(), 10.0 * a.sin(), (a * 6.0).sin())
        })
        .collect();
    let a = min_area_triangulation_any(&points).expect("cap a");
    let b = min_area_triangulation_any(&points).expect("cap b");
    assert_eq!(a, b, "the membrane triangulation must be deterministic");
}

/// Pre-cleaning drops dangling lone/needle triangles and reports the count; a
/// clean mesh passes through untouched (`None`) so callers stay byte-stable.
#[test]
fn heal_boundary_rims_drops_lone_triangles_and_leaves_clean_meshes_alone() {
    // A watertight-ish strip (two triangles sharing an edge) plus a lone
    // free-standing triangle nick.
    let good = MeshEditBuffers {
        vertices: vec![
            EditVertex::at([0.0, 0.0, 0.0]),
            EditVertex::at([1.0, 0.0, 0.0]),
            EditVertex::at([1.0, 1.0, 0.0]),
            EditVertex::at([0.0, 1.0, 0.0]),
            // Lone triangle far away.
            EditVertex::at([5.0, 5.0, 0.0]),
            EditVertex::at([6.0, 5.0, 0.0]),
            EditVertex::at([5.0, 6.0, 0.0]),
        ],
        indices: vec![0, 1, 2, 0, 2, 3, 4, 5, 6],
        topology: MeshTopology::TriangleMesh,
    };
    let outcome = heal_boundary_rims(&good).expect("the lone triangle is a defect to heal");
    assert!(outcome.healed >= 1, "at least the lone triangle is healed");
    assert_eq!(
        outcome.mesh.triangle_count(),
        2,
        "the lone triangle is dropped, the quad kept"
    );
    // The keep-mask must mark the two quad faces and drop the lone one.
    assert_eq!(outcome.keep, vec![true, true, false]);

    // A clean closed tetrahedron heals to nothing (None), keeping callers
    // byte-for-byte unchanged.
    let tetra = MeshEditBuffers {
        vertices: vec![
            EditVertex::at([0.0, 0.0, 0.0]),
            EditVertex::at([1.0, 0.0, 0.0]),
            EditVertex::at([0.0, 1.0, 0.0]),
            EditVertex::at([0.0, 0.0, 1.0]),
        ],
        indices: vec![0, 2, 1, 0, 1, 3, 1, 2, 3, 0, 3, 2],
        topology: MeshTopology::TriangleMesh,
    };
    assert!(
        heal_boundary_rims(&tetra).is_none(),
        "a clean closed mesh must not be altered"
    );
}

/// Healing is OFF by default, so the existing (repair, default-option) callers
/// keep the legacy walk: a lone-triangle nick is refused as damaged, not
/// silently healed away.
#[test]
fn close_holes_without_healing_keeps_legacy_behavior() {
    let mesh = MeshEditBuffers {
        vertices: vec![
            EditVertex::at([0.0, 0.0, 0.0]),
            EditVertex::at([1.0, 0.0, 0.0]),
            EditVertex::at([0.0, 1.0, 0.0]),
        ],
        indices: vec![0, 1, 2],
        topology: MeshTopology::TriangleMesh,
    };
    let report = fill_holes(&mesh, None, MeshEditOptions::default())
        .expect("fill")
        .report;
    assert_eq!(report.healed_rims, 0, "default options must not heal");
    // A lone triangle's only cap is its reverse twin, so it is honestly refused.
    assert_eq!(report.filled_holes, 0);
}

/// End-to-end determinism: the same socket closes to the identical mesh twice.
#[test]
fn tooth_socket_close_is_deterministic() {
    let mesh = dome_with_tooth(120, 70);
    let del = FaceSelection::new(lasso_delete_mask(&mesh));
    let deleted = delete_selected_faces(
        &mesh,
        &del,
        MeshEditOptions {
            compact_vertices: true,
            ..MeshEditOptions::default()
        },
    )
    .expect("delete");
    let rim = FaceSelection::new(rim_selection_mask(&deleted.mesh));
    let opts = MeshEditOptions {
        compact_vertices: true,
        heal_boundary_rims: true,
        ..MeshEditOptions::default()
    };
    let a = fill_holes(&deleted.mesh, Some(&rim), opts).expect("close a");
    let b = fill_holes(&deleted.mesh, Some(&rim), opts).expect("close b");
    assert_eq!(a.mesh.indices, b.mesh.indices);
    assert_eq!(a.mesh.vertices, b.mesh.vertices);
    assert_eq!(a.report, b.report);
}

/// The 3D simplicity discriminator, driven by the local edge-scale tube, keeps
/// an honest wiggly non-planar rim (highly non-uniform edge lengths, no
/// self-approach) simple, while still refusing a genuine self-crossing.
#[test]
fn rim_simplicity_passes_honest_wiggles_and_catches_crossings() {
    // A wavy, out-of-plane ring with strongly varying edge lengths (angular
    // clustering + radius/height wiggle) but no self-approach: honest, simple.
    let n = 60;
    let honest: Vec<Vec3> = (0..n)
        .map(|k| {
            // Non-uniform angle → edge lengths span an order of magnitude, so
            // the local-scale tube (not a global mean) governs each pair.
            let t = (k as f32 + 0.45 * (k as f32).sin()) / n as f32;
            let a = t * std::f32::consts::TAU;
            let r = 10.0 + 1.5 * (a * 4.0).sin();
            Vec3::new(r * a.cos(), r * a.sin(), 1.2 * (a * 3.0).cos())
        })
        .collect();
    assert!(
        rim_is_simple_3d(&honest),
        "an honest wiggly rim with non-uniform edges must stay simple"
    );

    // A genuine hourglass crossing (edges 0-1 and 2-3 intersect) stays refused.
    let crossing = vec![
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(1.0, 1.0, 0.0),
        Vec3::new(1.0, 0.0, 0.0),
        Vec3::new(0.0, 1.0, 0.0),
    ];
    assert!(
        !rim_is_simple_3d(&crossing),
        "a real self-crossing must still be refused"
    );
}

/// Issue #9 regression: a rim well past the OLD 160-edge interpolation ceiling
/// must still get the refined interpolated cap — not the raw min-area
/// membrane, whose near-folded creases were the "sharp spike-like artifacts"
/// a technician reported at the cap↔mesh transition after a lasso cut.
/// Quality is asserted the way the artifact shows: via dihedral angles across
/// the cap and its seam.
#[test]
fn large_rim_gets_interpolated_cap_without_spikes() {
    // A dense curved sheet with a big round hole: the rim lands well past the
    // retired 160-edge ceiling and stays far from the sheet's outer border, so
    // the whole-mesh path closes it while the border guard keeps the sheet
    // edge open.
    let (nu, nv) = (160, 110);
    let mesh = dome_with_tooth(nu, nv);
    let (hole_x, hole_y, hole_r) = (0.15_f32 * 20.0, 0.0_f32, 3.4_f32);
    let mask: Vec<bool> = mesh
        .indices
        .chunks_exact(3)
        .map(|t| {
            let c = (Vec3::from_array(mesh.vertices[t[0] as usize].position)
                + Vec3::from_array(mesh.vertices[t[1] as usize].position)
                + Vec3::from_array(mesh.vertices[t[2] as usize].position))
                / 3.0;
            let (dx, dy) = (c.x - hole_x, c.y - hole_y);
            (dx * dx + dy * dy).sqrt() < hole_r
        })
        .collect();
    let selection = FaceSelection::new(mask);
    let cut = delete_selected_faces(&mesh, &selection, MeshEditOptions::default())
        .expect("cut a round hole")
        .mesh;

    // Sanity: the rim really is past the old ceiling (the regression trigger).
    let mut edge_use: HashMap<(u32, u32), i32> = HashMap::new();
    for t in cut.indices.chunks_exact(3) {
        for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            *edge_use.entry((a.min(b), a.max(b))).or_default() += 1;
        }
    }
    let hole_rim_edges = edge_use
        .iter()
        .filter(|(&(a, b), &count)| {
            let mid = (Vec3::from_array(cut.vertices[a as usize].position)
                + Vec3::from_array(cut.vertices[b as usize].position))
                * 0.5;
            let (dx, dy) = (mid.x - hole_x, mid.y - hole_y);
            count == 1 && (dx * dx + dy * dy).sqrt() < hole_r * 1.5
        })
        .count();
    assert!(
        hole_rim_edges > 200,
        "fixture must produce a >200-edge rim, got {hole_rim_edges}"
    );

    let input_vertices = cut.vertices.len();
    let input_triangles = cut.triangle_count();
    let result = fill_holes(&cut, None, MeshEditOptions::default()).expect("large-rim fill");
    assert_eq!(result.report.filled_holes, 1, "the hole must close");
    assert_eq!(
        result.report.skipped_border_rims, 1,
        "the sheet border must stay open"
    );
    assert!(
        result.mesh.vertices.len() > input_vertices,
        "an interpolated cap generates interior vertices; a membrane (the old \
         >160-edge behavior) generates none"
    );

    // Spike metric, exactly how the artifact shows in a slicer: dihedral
    // angles on every edge owned by at least one NEW (cap) triangle.
    let positions: Vec<Vec3> = result
        .mesh
        .vertices
        .iter()
        .map(|v| Vec3::from_array(v.position))
        .collect();
    let normal = |t: &[u32]| -> Vec3 {
        let [a, b, c] = [
            positions[t[0] as usize],
            positions[t[1] as usize],
            positions[t[2] as usize],
        ];
        (b - a).cross(c - a).normalize_or_zero()
    };
    let mut owners: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
    for (index, t) in result.mesh.indices.chunks_exact(3).enumerate() {
        for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            owners.entry((a.min(b), a.max(b))).or_default().push(index);
        }
    }
    let mut worst = 0.0_f32;
    for slots in owners.values() {
        let [t1, t2] = match slots.as_slice() {
            [t1, t2] => [*t1, *t2],
            _ => continue,
        };
        if t1 < input_triangles && t2 < input_triangles {
            continue; // Original surface anatomy is not the cap's doing.
        }
        let (n1, n2) = (
            normal(&result.mesh.indices[t1 * 3..t1 * 3 + 3]),
            normal(&result.mesh.indices[t2 * 3..t2 * 3 + 3]),
        );
        if n1 == Vec3::ZERO || n2 == Vec3::ZERO {
            continue;
        }
        let angle = n1.dot(n2).clamp(-1.0, 1.0).acos().to_degrees();
        worst = worst.max(angle);
    }
    assert!(
        worst < 45.0,
        "cap/seam dihedral must stay spike-free, worst was {worst:.1} degrees"
    );
}
