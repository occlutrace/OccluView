//! A marked fill works on what the mark reaches: which rims it closes, what
//! it leaves alone, and what the cap looks like where the scan was cut.

use crate::{
    fill_selected_holes, EditVertex, FaceSelection, MeshEditBuffers, MeshEditOptions, MeshTopology,
};
use glam::Vec3;
use std::collections::{HashMap, HashSet};

/// The production Close Holes options (healed cut line, compacted vertices).
fn close_holes_options() -> MeshEditOptions {
    MeshEditOptions {
        compact_vertices: true,
        heal_boundary_rims: true,
        ..MeshEditOptions::default()
    }
}

/// A flat sheet of `nu` by `nv` quads, each split in two, with the quads for
/// which `open` holds left out. Returns the mesh and, per triangle, the quad
/// it came from.
fn sheet(
    nu: usize,
    nv: usize,
    open: impl Fn(usize, usize) -> bool,
) -> (MeshEditBuffers, Vec<(usize, usize)>) {
    let vertices = (0..=nv)
        .flat_map(|j| (0..=nu).map(move |i| EditVertex::at([i as f32, j as f32, 0.0])))
        .collect();
    let at = |i: usize, j: usize| (j * (nu + 1) + i) as u32;
    let mut indices = Vec::new();
    let mut quads = Vec::new();
    for j in 0..nv {
        for i in 0..nu {
            if open(i, j) {
                continue;
            }
            indices.extend_from_slice(&[at(i, j), at(i + 1, j), at(i + 1, j + 1)]);
            indices.extend_from_slice(&[at(i, j), at(i + 1, j + 1), at(i, j + 1)]);
            quads.extend([(i, j); 2]);
        }
    }
    let mesh = MeshEditBuffers {
        vertices,
        indices,
        topology: MeshTopology::TriangleMesh,
    };
    (mesh, quads)
}

/// One vertex per triangle corner, the way an STL arrives.
fn explode_to_soup(mesh: &MeshEditBuffers) -> MeshEditBuffers {
    let vertices: Vec<EditVertex> = mesh
        .indices
        .iter()
        .map(|&index| mesh.vertices[index as usize])
        .collect();
    MeshEditBuffers {
        indices: (0..vertices.len() as u32).collect(),
        vertices,
        topology: mesh.topology,
    }
}

/// Boundary half-edges of the surface, with coincident corners taken as one.
fn open_edges(mesh: &MeshEditBuffers) -> usize {
    let key = |index: u32| mesh.vertices[index as usize].position.map(f32::to_bits);
    let mut directed = HashSet::new();
    for triangle in mesh.indices.as_chunks::<3>().0 {
        for side in 0..3 {
            directed.insert((key(triangle[side]), key(triangle[(side + 1) % 3])));
        }
    }
    directed
        .iter()
        .filter(|(from, to)| !directed.contains(&(*to, *from)))
        .count()
}

/// A slot 40 quads long in a sheet. `marked_to` is the column the mark stops
/// short of; the mark starts well clear of the sheet's own border.
fn slot_and_mark(marked_to: usize) -> (MeshEditBuffers, FaceSelection) {
    let (mesh, quads) = sheet(64, 24, |i, j| {
        (12..52).contains(&i) && (10..14).contains(&j)
    });
    let mark = quads
        .iter()
        .map(|&(i, j)| (4..marked_to).contains(&i) && (4..20).contains(&j))
        .collect();
    (mesh, FaceSelection::new(mark))
}

/// A lasso over six tenths of a hole marks more than half of its rim, and the
/// rest of the rim runs out of the mark. The hole is not what the operator
/// selected: it stays open, and the report says a rim was only partly marked.
#[test]
fn a_hole_that_runs_out_of_the_mark_stays_open() {
    let (mesh, mark) = slot_and_mark(36);
    let result = fill_selected_holes(&mesh, &mark, close_holes_options()).expect("fill");
    assert_eq!(
        result.report.filled_holes, 0,
        "the slot is only partly marked"
    );
    assert_eq!(result.report.skipped_partial_rims, 1);
    assert_eq!(result.report.skipped_damaged_rims, 0);
    assert!(result.report.warnings.is_empty());
    assert_eq!(result.mesh, mesh, "nothing changes");
}

/// The same hole with the mark stopping a few faces short of its far end: the
/// faces a lasso misses at its edge do not keep the hole open.
#[test]
fn a_hole_the_mark_all_but_covers_closes() {
    let (mesh, mark) = slot_and_mark(48);
    let before = open_edges(&mesh);
    let result = fill_selected_holes(&mesh, &mark, close_holes_options()).expect("fill");
    assert_eq!(result.report.filled_holes, 1);
    assert_eq!(result.report.skipped_partial_rims, 0);
    // The slot's rim is gone; the sheet's own border is all that is open.
    assert_eq!(before - open_edges(&result.mesh), 2 * (40 + 4));
}

/// A mark is local and so is the work: on a scan that arrives as a soup, the
/// faces the mark does not reach come back as they went in, corner for
/// corner, instead of the whole scan being welded and its normals recomputed
/// for one hole.
#[test]
fn faces_the_mark_does_not_reach_come_back_untouched() {
    let (welded, quads) = sheet(60, 40, |i, j| (8..12).contains(&i) && (8..12).contains(&j));
    let mut soup = explode_to_soup(&welded);
    // Normals a fill would never produce, to see which ones it rewrote.
    for vertex in &mut soup.vertices {
        vertex.normal = [0.6, 0.0, 0.8];
    }
    let mark: Vec<bool> = quads
        .iter()
        .map(|&(i, j)| (5..15).contains(&i) && (5..15).contains(&j))
        .collect();
    let result = fill_selected_holes(
        &soup,
        &FaceSelection::new(mark.clone()),
        MeshEditOptions {
            compact_vertices: false,
            ..close_holes_options()
        },
    )
    .expect("fill");
    assert_eq!(result.report.filled_holes, 1);

    let far: Vec<usize> = (0..quads.len())
        .filter(|&triangle| quads[triangle].0 > 30)
        .collect();
    assert!(far.len() > 1_000);
    for triangle in far {
        let corners = triangle * 3..triangle * 3 + 3;
        assert_eq!(
            result.mesh.indices[corners.clone()],
            soup.indices[corners.clone()]
        );
        for &corner in &soup.indices[corners] {
            assert_eq!(
                result.mesh.vertices[corner as usize], soup.vertices[corner as usize],
                "a face far from the mark was rewritten"
            );
        }
    }
    // Where the fill worked, the surface is welded and closed.
    assert_eq!(open_edges(&result.mesh), open_edges(&welded) - 4 * 4);
}

/// An upright tube of `rings` rows of quads, open at both ends.
fn tube(sectors: usize, rings: usize, radius: f32, row_height: f32) -> MeshEditBuffers {
    let vertices = (0..=rings)
        .flat_map(|row| {
            (0..sectors).map(move |sector| {
                let angle = std::f32::consts::TAU * (sector as f32) / (sectors as f32);
                EditVertex::at([
                    radius * angle.cos(),
                    radius * angle.sin(),
                    row as f32 * row_height,
                ])
            })
        })
        .collect();
    let at = |sector: usize, row: usize| (row * sectors + sector % sectors) as u32;
    let mut indices = Vec::new();
    for row in 0..rings {
        for sector in 0..sectors {
            indices.extend_from_slice(&[
                at(sector, row),
                at(sector + 1, row),
                at(sector + 1, row + 1),
            ]);
            indices.extend_from_slice(&[
                at(sector, row),
                at(sector + 1, row + 1),
                at(sector, row + 1),
            ]);
        }
    }
    MeshEditBuffers {
        vertices,
        indices,
        topology: MeshTopology::TriangleMesh,
    }
}

/// A tooth cut square across is a tube of wall with an open top. Closing it
/// has to carry the wall on: the cap rises from the rim the way the wall
/// arrives and rounds over, with no crease at the seam and no flat lid.
#[test]
fn a_cut_tooth_is_closed_by_a_dome_that_follows_its_wall() {
    let (sectors, rings, radius, row_height) = (96, 12, 4.0_f32, 0.26_f32);
    let mesh = tube(sectors, rings, radius, row_height);
    let top = rings as f32 * row_height;
    // The operator marks the wall below the cut, not the tube's other end.
    let mark: Vec<bool> = (0..mesh.triangle_count())
        .map(|triangle| triangle / (2 * sectors) >= rings - 4)
        .collect();
    // Vertices keep their places, so the rim can be found again below.
    let result = fill_selected_holes(
        &mesh,
        &FaceSelection::new(mark),
        MeshEditOptions {
            compact_vertices: false,
            ..close_holes_options()
        },
    )
    .expect("fill");
    assert_eq!(result.report.filled_holes, 1, "the cut end closes");
    assert_eq!(result.report.skipped_damaged_rims, 0);

    let cap: Vec<Vec3> = result.mesh.vertices[mesh.vertices.len()..]
        .iter()
        .map(|vertex| Vec3::from_array(vertex.position))
        .collect();
    assert!(
        cap.len() > 500,
        "a cap at the wall's density: {}",
        cap.len()
    );
    let rise = cap.iter().map(|point| point.z).fold(f32::MIN, f32::max) - top;
    assert!(
        rise > 0.3 * radius,
        "the cap rose {rise:.2} mm over a {radius} mm tooth: a lid, not a dome"
    );
    for point in &cap {
        assert!(
            point.z > top - 1e-3,
            "the cap dips below the cut: {point:?}"
        );
        assert!(
            point.truncate().length() < radius * 1.02,
            "the cap bulges past the wall: {point:?}"
        );
    }

    // No crease at the seam: across every edge of the old rim the wall and
    // the cap go on within a few degrees of each other.
    let normal = |triangle: &[u32]| {
        let [a, b, c] = [triangle[0], triangle[1], triangle[2]]
            .map(|index| Vec3::from_array(result.mesh.vertices[index as usize].position));
        (b - a).cross(c - a).normalize()
    };
    let mut sides: HashMap<(u32, u32), Vec<Vec3>> = HashMap::new();
    for triangle in result.mesh.indices.as_chunks::<3>().0 {
        for side in 0..3 {
            let (from, to) = (triangle[side], triangle[(side + 1) % 3]);
            sides
                .entry((from.min(to), from.max(to)))
                .or_default()
                .push(normal(triangle));
        }
    }
    let top_row = (rings * sectors) as u32;
    for sector in 0..sectors as u32 {
        let (from, to) = (top_row + sector, top_row + (sector + 1) % sectors as u32);
        let normals = &sides[&(from.min(to), from.max(to))];
        assert_eq!(normals.len(), 2, "the seam is closed");
        let turn = normals[0]
            .dot(normals[1])
            .clamp(-1.0, 1.0)
            .acos()
            .to_degrees();
        assert!(turn < 15.0, "a {turn:.0} degree crease at the seam");
    }
}

/// A lasso cut leaves a saw: faces that hang on the rim by one edge and stick
/// into the hole. The cap does not wrap round them and does not lay a triangle
/// back onto one; they go, the rim runs along their roots, and the hole
/// closes with a cap that shares no triangle with the scan.
#[test]
fn the_teeth_of_a_cut_line_go_with_the_cap() {
    // A hole whose left edge keeps the upper triangle of every other quad:
    // each hangs by its left edge and has two sides on the rim.
    let (mut mesh, _) = sheet(30, 30, |i, j| {
        (10..20).contains(&i) && (10..20).contains(&j)
    });
    let at = |i: usize, j: usize| (j * 31 + i) as u32;
    let teeth: Vec<[u32; 3]> = (10..20)
        .step_by(2)
        .map(|j| [at(10, j), at(11, j + 1), at(10, j + 1)])
        .collect();
    let surface = mesh.triangle_count();
    for tooth in &teeth {
        mesh.indices.extend_from_slice(tooth);
    }
    let mark = FaceSelection::new(
        mesh.indices
            .as_chunks::<3>()
            .0
            .iter()
            .map(|triangle| {
                triangle.iter().all(|&index| {
                    let [x, y, _] = mesh.vertices[index as usize].position;
                    (6.0..=24.0).contains(&x) && (6.0..=24.0).contains(&y)
                })
            })
            .collect(),
    );

    // Vertices keep their places, so the surface can be compared below.
    let result = fill_selected_holes(
        &mesh,
        &mark,
        MeshEditOptions {
            compact_vertices: false,
            ..close_holes_options()
        },
    )
    .expect("fill");
    assert_eq!(result.report.filled_holes, 1);
    assert_eq!(result.report.skipped_damaged_rims, 0);
    assert_eq!(
        result.report.removed_triangles,
        teeth.len(),
        "the teeth, and nothing of the surface behind them"
    );
    assert_eq!(result.report.healed_rims, 0, "a tooth is not a defect");
    assert_eq!(
        result.mesh.indices[..surface * 3],
        mesh.indices[..surface * 3],
        "the surface behind the teeth stays"
    );
    assert_eq!(
        open_edges(&result.mesh),
        4 * 30,
        "only the sheet's border is open"
    );

    // No two triangles on the same three vertices: no cap lies on the scan.
    let mut seen = HashSet::new();
    for triangle in result.mesh.indices.as_chunks::<3>().0 {
        let mut triple = *triangle;
        triple.sort_unstable();
        assert!(seen.insert(triple), "a triangle doubled: {triple:?}");
    }
}
