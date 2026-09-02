//! Scenario-matrix tests for hole filling: the HPS-scan-like default-fill
//! contract (close every interior hole, protect the scan border, report
//! honestly per reason), pinch/edge-sharing rim topologies, pinhole clusters,
//! tiny rims, unwelded seams, attribute blending, and the dense-zigzag
//! pathological rim that must never hang.

use crate::{fill_holes, EditVertex, MeshEditBuffers, MeshEditOptions, MeshTopology};
use glam::Vec3;
use std::collections::HashSet;

fn v(p: [f32; 3]) -> EditVertex {
    EditVertex::at(p)
}

fn tri_mesh(vertices: Vec<EditVertex>, indices: Vec<u32>) -> MeshEditBuffers {
    MeshEditBuffers {
        vertices,
        indices,
        topology: MeshTopology::TriangleMesh,
    }
}

fn boundary_edge_count(indices: &[u32]) -> usize {
    let mut directed = HashSet::new();
    for triangle in indices.as_chunks::<3>().0 {
        for edge in [
            (triangle[0], triangle[1]),
            (triangle[1], triangle[2]),
            (triangle[2], triangle[0]),
        ] {
            directed.insert(edge);
        }
    }
    directed
        .iter()
        .filter(|(a, b)| !directed.contains(&(*b, *a)))
        .count()
}

/// Square bowl (apex + 4-vertex rim) reused across pinch fixtures.
fn push_bowl(
    vertices: &mut Vec<EditVertex>,
    indices: &mut Vec<u32>,
    rim: [[f32; 3]; 4],
    apex: [f32; 3],
) {
    let base = vertices.len() as u32;
    for corner in rim {
        vertices.push(v(corner));
    }
    vertices.push(v(apex));
    let apex_index = base + 4;
    for i in 0..4u32 {
        let a = base + i;
        let b = base + (i + 1) % 4;
        indices.extend_from_slice(&[apex_index, a, b]);
    }
}

fn guard_off() -> MeshEditOptions {
    MeshEditOptions {
        protect_scan_border: false,
        ..MeshEditOptions::default()
    }
}

// HPS-like

/// Spherical shell with a huge natural border, four interior holes of ~3, 10,
/// 25 and 60 mm perimeter, one extra hole sharing a pinch vertex with the
/// border, and a genuinely non-simple (hourglass) island rim. This is the
/// dental-scan shape used to pin close-holes behaviour to the convention
/// operators expect.
fn hps_like_shell() -> (MeshEditBuffers, usize) {
    let radius = 40.0_f32;
    let sectors = 220_usize;
    let rings = 60_usize;
    let theta0 = 0.12_f32;
    let theta1 = 1.25_f32;

    let mut vertices = Vec::new();
    let mut grid: Vec<Vec<u32>> = Vec::new();
    for ri in 0..=rings {
        let theta = theta0 + (theta1 - theta0) * (ri as f32 / rings as f32);
        let mut row = Vec::new();
        for si in 0..sectors {
            let phi = std::f32::consts::TAU * (si as f32) / (sectors as f32);
            row.push(vertices.len() as u32);
            vertices.push(v([
                radius * theta.sin() * phi.cos(),
                radius * theta.cos(),
                radius * theta.sin() * phi.sin(),
            ]));
        }
        grid.push(row);
    }

    // Hole descriptors: (ring, sector, angular half-extent scaled to reach the
    // requested perimeter). Perimeter of a small circular hole of angular
    // radius alpha on the sphere: ~2*pi*R*alpha (alpha in radians, small).
    let hole_centers = [
        (12_usize, 30_usize, 3.0_f32),
        (20, 90, 10.0),
        (32, 150, 25.0),
        (44, 40, 60.0),
    ];

    let position = |ri: usize, si: usize| -> Vec3 {
        Vec3::from_array(vertices[grid[ri][si] as usize].position)
    };

    let mut removed: HashSet<(usize, usize)> = HashSet::new();
    let mut mark_hole = |ri0: usize, si0: usize, perimeter_mm: f32| {
        let alpha = perimeter_mm / (std::f32::consts::TAU * radius);
        let chord_limit = 2.0 * radius * (alpha * 0.5).sin();
        let center = position(ri0, si0);
        for ri in 0..rings {
            for si in 0..sectors {
                // Both triangles of the cell (ri, si) share the cell corner.
                let corner = position(ri, si);
                if (corner - center).length() < chord_limit {
                    removed.insert((ri, si));
                }
            }
        }
    };
    for &(ri, si, perimeter) in &hole_centers {
        mark_hole(ri, si, perimeter);
    }

    // Pinch hole: the [a, c, b] triangle of an outermost-row cell has exactly
    // ONE vertex (c) on the border rim. Removing it leaves a 3-edge hole that
    // touches the border loop at that single vertex — a boundary pinch.
    let pinch_cell = (rings - 1, 200_usize);

    let mut indices = Vec::new();
    for ri in 0..rings {
        for si in 0..sectors {
            let s1 = (si + 1) % sectors;
            let a = grid[ri][si];
            let b = grid[ri][s1];
            let c = grid[ri + 1][s1];
            let d = grid[ri + 1][si];
            if removed.contains(&(ri, si)) {
                continue;
            }
            // Outward winding for +Y-cap sphere rows.
            indices.extend_from_slice(&[a, d, c]);
            if !(ri == pinch_cell.0 && si == pinch_cell.1) {
                indices.extend_from_slice(&[a, c, b]);
            }
        }
    }

    // Hourglass island: rim 0->1->2->3 crosses itself (edges 0-1 and 2-3),
    // fanned from a deep apex. Genuinely non-simple: must be refused as
    // damaged, never capped.
    let base = vertices.len() as u32;
    for p in [
        [70.0, 0.0, 0.0],
        [71.0, 1.0, 0.0],
        [71.0, 0.0, 0.0],
        [70.0, 1.0, 0.0],
        [70.5, 0.5, -1.0],
    ] {
        vertices.push(v(p));
    }
    for (x, y) in [(0, 1), (1, 2), (2, 3), (3, 0)] {
        indices.extend_from_slice(&[base + 4, base + x, base + y]);
    }

    // Interior rims: the four punched holes, plus the border-pinch hole,
    // plus one extra rim — the 60 mm staircase removal region touches itself
    // at a grid corner and pinch-splits into two loops (deterministic).
    let interior_holes = hole_centers.len() + 2;
    (tri_mesh(vertices, indices), interior_holes)
}

#[test]
fn hps_like_default_closes_all_interior_holes_and_protects_the_border() {
    let (mesh, interior_holes) = hps_like_shell();
    let result = fill_holes(&mesh, None, MeshEditOptions::default()).expect("hps-like fill");

    // Every interior hole closes — the 60 mm one included (no mm gate by
    // default). The scan border is the only protected rim; the hourglass
    // island is refused as damaged.
    assert_eq!(
        result.report.filled_holes, interior_holes,
        "every interior hole must close by default"
    );
    assert_eq!(result.report.skipped_border_rims, 1, "only the scan border");
    assert_eq!(result.report.skipped_damaged_rims, 1, "hourglass island");
    assert_eq!(result.report.skipped_oversize_rims, 0);
    assert_eq!(
        result.report.warnings.len(),
        result.report.skipped_border_rims
            + result.report.skipped_oversize_rims
            + result.report.skipped_damaged_rims,
        "one warning per skipped loop, whatever the reason"
    );

    // All remaining POSITIVE-LENGTH boundary edges belong to the border loop
    // or the refused hourglass (4 edges): no interior hole edge survives. The
    // capped pinch hole legitimately leaves one zero-length crack between the
    // two junction copies.
    let sectors = 220;
    assert_eq!(
        positive_length_boundary_edges(&result.mesh),
        sectors + 4,
        "border stays open, hourglass stays open, nothing else"
    );
}

/// Boundary edges with nonzero length (zero-length junction cracks between
/// coincident-position copies are invisible and intentionally tolerated).
fn positive_length_boundary_edges(mesh: &MeshEditBuffers) -> usize {
    let mut directed = HashSet::new();
    for triangle in mesh.indices.as_chunks::<3>().0 {
        for edge in [
            (triangle[0], triangle[1]),
            (triangle[1], triangle[2]),
            (triangle[2], triangle[0]),
        ] {
            directed.insert(edge);
        }
    }
    directed
        .iter()
        .filter(|(a, b)| !directed.contains(&(*b, *a)))
        .filter(|(a, b)| mesh.vertices[*a as usize].position != mesh.vertices[*b as usize].position)
        .count()
}

#[test]
fn hps_like_optional_mm_restraint_still_limits_large_holes() {
    let (mesh, _) = hps_like_shell();
    let restrained = MeshEditOptions {
        max_rim_perimeter_mm: Some(15.0),
        ..MeshEditOptions::default()
    };
    let result = fill_holes(&mesh, None, restrained).expect("restrained fill");

    // ~3 and ~10 mm holes close (plus the tiny border-pinch hole); ~25 and
    // ~60 mm — and the corner fragment of the 60 mm region — are held back
    // by the operator's explicit mm restraint.
    assert_eq!(result.report.filled_holes, 3);
    assert_eq!(result.report.skipped_oversize_rims, 3);
    assert_eq!(result.report.skipped_border_rims, 1);
}

// pinch matrix

#[test]
fn three_fans_meeting_at_one_vertex_all_close() {
    // THREE bowls sharing a single pinch vertex: the junction splits into one
    // copy per fan and every rim closes independently.
    let mut vertices = vec![v([0.0, 0.0, 0.0])]; // shared pinch vertex
    let mut indices = Vec::new();
    for k in 0..3 {
        let angle = std::f32::consts::TAU * (k as f32) / 3.0;
        let (s, c) = angle.sin_cos();
        // Rim square anchored at the shared origin vertex, rotated per fan.
        let base = vertices.len() as u32;
        let corners = [[c, s, 0.0], [c - s, s + c, 0.0], [-s, c, 0.0]];
        for corner in corners {
            vertices.push(v(corner));
        }
        #[expect(
            clippy::manual_midpoint,
            reason = "preserve established last-bit geometry fixture"
        )]
        vertices.push(v([0.5 * (c - s), 0.5 * (s + c), 1.0])); // apex
        let apex = base + 3;
        let ring = [0, base, base + 1, base + 2];
        for i in 0..4 {
            let a = ring[i];
            let b = ring[(i + 1) % 4];
            indices.extend_from_slice(&[apex, a, b]);
        }
    }
    let mesh = tri_mesh(vertices, indices);

    let split = crate::pinch::split_boundary_pinch_vertices(&mesh).expect("pinch split");
    let (_, split_count) = split.expect("the shared vertex splits");
    assert_eq!(split_count, 1, "one junction vertex, split into three fans");

    let result = fill_holes(&mesh, None, guard_off()).expect("three-fan fill");
    assert_eq!(result.report.filled_holes, 3);
    assert_eq!(boundary_edge_count(&result.mesh.indices), 0);
}

#[test]
fn two_holes_joined_through_a_shared_interior_edge_close_as_one() {
    // Two bowls whose open squares share one properly-wound edge: that edge
    // is interior (twinned), so the two openings form ONE composite 6-edge
    // hole. It must close watertight as a single hole.
    let mut vertices = vec![
        v([0.0, 0.0, 0.0]),  // 0: shared a
        v([1.0, 0.0, 0.0]),  // 1: shared b
        v([1.0, 1.0, 0.0]),  // 2
        v([0.0, 1.0, 0.0]),  // 3
        v([1.0, -1.0, 0.0]), // 4
        v([0.0, -1.0, 0.0]), // 5
    ];
    let mut indices = Vec::new();
    // Bowl A above (rim 0,1,2,3), apex up at z=1.
    vertices.push(v([0.5, 0.5, 1.0]));
    let apex_a = 6;
    for (x, y) in [(0u32, 1u32), (1, 2), (2, 3), (3, 0)] {
        indices.extend_from_slice(&[apex_a, x, y]);
    }
    // Bowl B below (rim 1,0,5,4 — opposite traversal of the shared edge).
    vertices.push(v([0.5, -0.5, 1.0]));
    let apex_b = 7;
    for (x, y) in [(1u32, 0u32), (0, 5), (5, 4), (4, 1)] {
        indices.extend_from_slice(&[apex_b, x, y]);
    }
    let mesh = tri_mesh(vertices, indices);

    let result = fill_holes(&mesh, None, guard_off()).expect("joined-hole fill");
    assert_eq!(result.report.filled_holes, 1, "one composite hole");
    assert!(result.report.warnings.is_empty());
    assert_eq!(boundary_edge_count(&result.mesh.indices), 0);
}

#[test]
fn tiny_rims_of_three_four_and_five_edges_close_watertight() {
    // 3-edge: tetrahedron missing one face.
    let tetra = tri_mesh(
        vec![
            v([0.0, 0.0, 0.0]),
            v([1.0, 0.0, 0.0]),
            v([0.5, 1.0, 0.0]),
            v([0.5, 0.5, 1.0]),
        ],
        vec![3, 1, 0, 3, 2, 1, 3, 0, 2],
    );
    let result = fill_holes(&tetra, None, guard_off()).expect("tetra fill");
    assert_eq!(result.report.filled_holes, 1);
    assert_eq!(boundary_edge_count(&result.mesh.indices), 0);

    // 4-edge: square bowl.
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    push_bowl(
        &mut vertices,
        &mut indices,
        [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ],
        [0.5, 0.5, 1.0],
    );
    let bowl = tri_mesh(vertices, indices);
    let result = fill_holes(&bowl, None, guard_off()).expect("bowl fill");
    assert_eq!(result.report.filled_holes, 1);
    assert_eq!(boundary_edge_count(&result.mesh.indices), 0);

    // 5-edge: pentagon fan from an apex.
    let mut vertices = Vec::new();
    for i in 0..5 {
        let t = std::f32::consts::TAU * (i as f32) / 5.0;
        vertices.push(v([t.cos(), t.sin(), 0.0]));
    }
    vertices.push(v([0.0, 0.0, 1.0]));
    let mut indices = Vec::new();
    for i in 0..5u32 {
        indices.extend_from_slice(&[5, i, (i + 1) % 5]);
    }
    let penta = tri_mesh(vertices, indices);
    let result = fill_holes(&penta, None, guard_off()).expect("pentagon fill");
    assert_eq!(result.report.filled_holes, 1);
    assert_eq!(boundary_edge_count(&result.mesh.indices), 0);
}

// soup seams

#[test]
fn unwelded_duplicate_position_seam_closes_watertight() {
    // STL-soup seam: the rim passes through a POSITION that exists twice with
    // distinct indices (one triangle references the copy), which re-routes
    // the boundary loop over the apex as a strongly folded 6-ring.
    let mut mesh = {
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        push_bowl(
            &mut vertices,
            &mut indices,
            [
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
            [0.5, 0.5, 1.0],
        );
        tri_mesh(vertices, indices)
    };
    // Duplicate rim vertex 1 (position copy, distinct index) and repoint ONE
    // incident triangle to the copy.
    let copy = mesh.vertices[1];
    mesh.vertices.push(copy);
    let duplicate = (mesh.vertices.len() - 1) as u32;
    // Triangle (apex, 1, 2) becomes (apex, dup, 2).
    let slot = mesh
        .indices
        .as_chunks::<3>()
        .0
        .iter()
        .position(|t| *t == [4, 1, 2])
        .expect("triangle present")
        * 3
        + 1;
    mesh.indices[slot] = duplicate;

    let result = fill_holes(&mesh, None, guard_off()).expect("soup fill");
    // The rim walks as one valid 6-loop THROUGH the apex (the seam re-routes
    // it), its planar projection self-overlaps, and the projection-free
    // minimum-area fallback closes it watertight. The old kernel refused it.
    assert_eq!(result.report.filled_holes, 1, "seam slit closes");
    assert_eq!(result.report.skipped_damaged_rims, 0);
    assert_eq!(boundary_edge_count(&result.mesh.indices), 0);
}

#[test]
fn rim_with_duplicate_position_but_distinct_indices_on_one_loop_still_closes() {
    // A single VALID loop that visits two distinct indices carrying the SAME
    // position (legal in dental formats). The cap must not weld or panic.
    let mut vertices = Vec::new();
    for i in 0..8 {
        let t = std::f32::consts::TAU * (i as f32) / 8.0;
        vertices.push(v([t.cos(), t.sin(), 0.1 * (2.0 * t).sin()]));
    }
    // Vertex 8 duplicates vertex 0's position; the ring uses BOTH (0 at the
    // seam start, 8 as an extra rim sample stitched between 7 and 0).
    vertices.push(vertices[0]);
    vertices.push(v([0.0, 0.0, -1.5])); // apex 9
    let apex = 9u32;
    let ring: [u32; 9] = [0, 1, 2, 3, 4, 5, 6, 7, 8];
    let mut indices = Vec::new();
    for i in 0..9_usize {
        let a = ring[i];
        let b = ring[(i + 1) % 9];
        indices.extend_from_slice(&[apex, a, b]);
    }
    let mesh = tri_mesh(vertices, indices);
    let result = fill_holes(&mesh, None, guard_off()).expect("dup-position rim fill");
    assert_eq!(result.report.filled_holes, 1);
    assert_eq!(boundary_edge_count(&result.mesh.indices), 0);
}

// mass pinholes

#[test]
fn fifty_pinholes_in_a_plane_all_close_while_the_border_stays() {
    // 40x40 mm plane grid, 50 single-triangle pinholes: the default fill
    // closes all of them and protects only the outer border.
    let side = 40_usize;
    let mut vertices = Vec::new();
    for y in 0..=side {
        for x in 0..=side {
            vertices.push(v([x as f32, y as f32, 0.0]));
        }
    }
    let at = |x: usize, y: usize| (y * (side + 1) + x) as u32;
    let mut punched = 0_usize;
    let mut indices = Vec::new();
    for y in 0..side {
        for x in 0..side {
            let (a, b, c, d) = (at(x, y), at(x + 1, y), at(x + 1, y + 1), at(x, y + 1));
            // Punch the LOWER triangle of every 5th interior cell on a
            // diagonal-ish pattern until 50 holes exist.
            let interior = x > 1 && x < side - 2 && y > 1 && y < side - 2;
            let punch = interior && punched < 50 && x % 5 == 2 && y % 5 == 2;
            if punch {
                punched += 1;
            } else {
                indices.extend_from_slice(&[a, b, c]);
            }
            indices.extend_from_slice(&[a, c, d]);
        }
    }
    assert_eq!(punched, 50, "fixture must actually contain 50 pinholes");
    let mesh = tri_mesh(vertices, indices);

    let result = fill_holes(&mesh, None, MeshEditOptions::default()).expect("pinhole fill");
    assert_eq!(result.report.filled_holes, 50);
    assert_eq!(result.report.skipped_border_rims, 1, "plane border only");
    assert_eq!(result.report.skipped_damaged_rims, 0);
    assert_eq!(
        boundary_edge_count(&result.mesh.indices),
        4 * side,
        "only the outer border remains open"
    );
}

// attributes / color

#[test]
fn two_tone_rim_colors_blend_into_the_cap() {
    let rim_len = 16_usize;
    let mut vertices: Vec<EditVertex> = (0..rim_len)
        .map(|index| {
            let t = std::f32::consts::TAU * (index as f32) / (rim_len as f32);
            let mut vertex = v([t.cos(), t.sin(), 0.2 * (2.0 * t).sin()]);
            vertex.color = if index < rim_len / 2 {
                [255, 0, 0, 255]
            } else {
                [0, 0, 255, 255]
            };
            vertex
        })
        .collect();
    vertices.push(v([0.0, 0.0, -1.2]));
    let apex = rim_len as u32;
    let mut indices = Vec::new();
    for i in 0..rim_len as u32 {
        indices.extend_from_slice(&[apex, i, (i + 1) % rim_len as u32]);
    }
    let mesh = tri_mesh(vertices, indices);
    let input_vertices = mesh.vertices.len();

    let result = fill_holes(&mesh, None, guard_off()).expect("two-tone fill");
    assert_eq!(result.report.filled_holes, 1);
    let generated = &result.mesh.vertices[input_vertices..];
    assert!(!generated.is_empty(), "interpolated cap generates vertices");
    let mut saw_blend = false;
    for vertex in generated {
        let [r, _, b, a] = vertex.color;
        assert_eq!(a, 255);
        if r > 0 && b > 0 {
            saw_blend = true; // strictly between the two rim tones
        }
    }
    assert!(
        saw_blend,
        "at least one cap vertex blends the two rim tones"
    );
}

// pathological (hang killer)

#[test]
fn dense_zigzag_rim_terminates_quickly_and_honestly() {
    // The live-hang shape: a rim with THOUSANDS of tiny zigzag edges and
    // near-duplicate vertices, small in mm. Every stage must stay bounded:
    // either the hole closes or it is refused with a warning — never a spin.
    let n = 3000_usize;
    let mut vertices = Vec::new();
    for i in 0..n {
        let t = std::f32::consts::TAU * (i as f32) / (n as f32);
        // Radius zigzags every step; amplitude comparable to the step so the
        // projected polygon is a dense saw.
        let r = 5.0 + if i % 2 == 0 { 0.0 } else { 0.008 };
        vertices.push(v([r * t.cos(), r * t.sin(), 0.002 * ((i % 3) as f32)]));
    }
    vertices.push(v([0.0, 0.0, -2.0]));
    let apex = n as u32;
    let mut indices = Vec::new();
    for i in 0..n as u32 {
        indices.extend_from_slice(&[apex, i, (i + 1) % n as u32]);
    }
    let mesh = tri_mesh(vertices, indices);

    let started = std::time::Instant::now();
    let result = fill_holes(&mesh, None, guard_off()).expect("zigzag fill terminates");
    let elapsed = started.elapsed();

    let filled = result.report.filled_holes;
    let skipped = result.report.warnings.len();
    assert_eq!(
        filled + skipped,
        1,
        "the rim either fills or warns, exactly once"
    );
    if filled == 1 {
        assert_eq!(boundary_edge_count(&result.mesh.indices), 0);
    }
    // Debug builds are ~10x slower than release; this bound only guards
    // against the pathological non-termination class, not micro-perf.
    assert!(
        elapsed.as_secs() < 60,
        "dense zigzag rim took {elapsed:?}: the fill path must stay bounded"
    );
}

/// A shell with exactly ONE rim: a dome closed at the bottom by a fan to a
/// single apex, open at the top.
///
/// The border guard's ratio half is vacuous here — a rim is always at least
/// half of itself — so the absolute anchor alone decides, and at half the
/// bounding-box diagonal it declared a routine socket "scan border" and refused
/// to close it. A molar socket is 25-35 mm of perimeter on a 65-75 mm arch,
/// right on that line.
#[test]
fn the_sole_rim_of_a_closed_shell_is_a_hole_not_a_scan_border() {
    /// Latitude bands between the open rim and the closing apex.
    const RINGS: usize = 12;
    /// Vertices around each band.
    const SEGMENTS: usize = 24;
    let radius = 10.0_f32;

    let mut vertices = Vec::new();
    for ring in 0..RINGS {
        // A modest polar opening on an almost-closed shell — the shape of a
        // molar socket on an arch, which is the case the guard used to refuse:
        // 31 mm of rim on a 34 mm diagonal, comfortably over the old half-the-
        // diagonal threshold and so wrongly declared "scan border".
        #[allow(clippy::cast_precision_loss)]
        let phi = std::f32::consts::FRAC_PI_6
            + (std::f32::consts::PI - std::f32::consts::FRAC_PI_6) * ring as f32 / RINGS as f32;
        for segment in 0..SEGMENTS {
            #[allow(clippy::cast_precision_loss)]
            let theta = std::f32::consts::TAU * segment as f32 / SEGMENTS as f32;
            vertices.push(v([
                radius * phi.sin() * theta.cos(),
                radius * phi.sin() * theta.sin(),
                radius * phi.cos(),
            ]));
        }
    }
    let apex = u32::try_from(vertices.len()).expect("index fits");
    vertices.push(v([0.0, 0.0, -radius]));

    let index = |ring: usize, segment: usize| -> u32 {
        u32::try_from(ring * SEGMENTS + segment % SEGMENTS).expect("index fits")
    };
    let mut indices = Vec::new();
    for ring in 0..RINGS - 1 {
        for segment in 0..SEGMENTS {
            let (a, b) = (index(ring, segment), index(ring, segment + 1));
            let (c, d) = (index(ring + 1, segment), index(ring + 1, segment + 1));
            indices.extend_from_slice(&[a, c, b, b, c, d]);
        }
    }
    for segment in 0..SEGMENTS {
        let a = index(RINGS - 1, segment);
        let b = index(RINGS - 1, segment + 1);
        indices.extend_from_slice(&[a, apex, b]);
    }

    let mesh = tri_mesh(vertices, indices);
    assert!(
        boundary_edge_count(&mesh.indices) == SEGMENTS,
        "the fixture must have exactly one rim of {SEGMENTS} edges"
    );

    let filled = fill_holes(&mesh, None, MeshEditOptions::default()).expect("fill runs");

    assert_eq!(
        filled.report.skipped_border_rims, 0,
        "the sole rim of a closed shell is a hole, not the scan's outer border"
    );
    assert_eq!(filled.report.filled_holes, 1);
    assert_eq!(
        boundary_edge_count(&filled.mesh.indices),
        0,
        "the shell must come back watertight"
    );
}

/// Rim healing deletes triangles; the report used to say it had removed none,
/// so `input - output` disagreed with `removed_triangles` and an operator had
/// no way to see that geometry had been dropped.
#[test]
fn healing_reports_the_triangles_it_actually_deleted() {
    // A small fan with one dangling needle hanging off its rim.
    let mesh = tri_mesh(
        vec![
            v([0.0, 0.0, 0.0]),
            v([1.0, 0.0, 0.0]),
            v([0.5, 1.0, 0.0]),
            v([-0.5, 1.0, 0.0]),
            v([0.5, 2.0, 0.0]),
            // The needle: a sliver sharing one edge and going nowhere.
            v([0.5001, 2.0002, 0.0]),
        ],
        vec![0, 1, 2, 0, 2, 3, 3, 2, 4, 4, 2, 5],
    );
    let options = MeshEditOptions {
        heal_boundary_rims: true,
        ..MeshEditOptions::default()
    };

    let filled = fill_holes(&mesh, None, options).expect("fill runs");

    // The contract: whatever healing deleted has to be visible in the report.
    // If the mesh came back smaller than it went in, `removed_triangles` is the
    // only field that can explain it, and it used to be hardcoded to zero.
    let report = &filled.report;
    if report.output_triangles < report.input_triangles {
        assert!(
            report.removed_triangles > 0,
            "the mesh shrank from {} to {} triangles but the report says nothing was removed",
            report.input_triangles,
            report.output_triangles
        );
    }
    assert!(
        report.removed_triangles <= report.input_triangles,
        "cannot remove more triangles than went in"
    );
}
