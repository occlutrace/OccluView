use super::*;

fn wavy_fan_mesh(rim_color: [u8; 4]) -> MeshEditBuffers {
    let rim_len = 12usize;
    let mut vertices: Vec<EditVertex> = (0..rim_len)
        .map(|index| {
            let theta = std::f32::consts::TAU * (index as f32) / (rim_len as f32);
            EditVertex {
                color: rim_color,
                ..v([theta.cos(), theta.sin(), 0.3 * (2.0 * theta).sin()])
            }
        })
        .collect();
    vertices.push(v([0.0, 0.0, -1.5]));
    let apex = rim_len;
    let mut indices = Vec::new();
    for index in 0..rim_len {
        let next = (index + 1) % rim_len;
        indices.extend_from_slice(&[apex as u32, index as u32, next as u32]);
    }
    let mut buffers = MeshEditBuffers {
        vertices,
        indices,
        topology: MeshTopology::TriangleMesh,
    };
    recompute_all_normals(&mut buffers.vertices, &buffers.indices).expect("seed normals");
    buffers
}

fn boundary_edge_count(indices: &[u32]) -> usize {
    let mut directed = std::collections::HashSet::new();
    for triangle in indices.chunks_exact(3) {
        for (a, b) in [
            (triangle[0], triangle[1]),
            (triangle[1], triangle[2]),
            (triangle[2], triangle[0]),
        ] {
            directed.insert((a, b));
        }
    }
    directed
        .iter()
        .filter(|(a, b)| !directed.contains(&(*b, *a)))
        .count()
}

#[test]
fn fill_holes_interpolated_cap_is_watertight_and_manifold() {
    let mesh = wavy_fan_mesh([255, 255, 255, 255]);
    let input_vertices = mesh.vertices.len();
    let options = MeshEditOptions {
        protect_scan_border: false, // lone-fan fixture: its rim is no border
        ..MeshEditOptions::default()
    };
    let result = fill_holes(&mesh, None, options).expect("interpolated fill");

    assert_eq!(result.report.filled_holes, 1);
    // The cap generated interior vertices (density-matched refinement).
    assert!(result.mesh.vertices.len() > input_vertices);
    // Watertight by construction: no directed edge is left without its twin.
    assert_eq!(boundary_edge_count(&result.mesh.indices), 0);
    // Reverse-twin invariant: no directed edge is emitted twice (manifold).
    let mut directed = std::collections::HashSet::new();
    for triangle in result.mesh.indices.chunks_exact(3) {
        for (a, b) in [
            (triangle[0], triangle[1]),
            (triangle[1], triangle[2]),
            (triangle[2], triangle[0]),
        ] {
            assert!(directed.insert((a, b)), "duplicated directed edge {a}->{b}");
        }
    }
}

/// Interior valence stays bounded: the ear-clip fan must be regularized into a
/// Delaunay-quality patch, NOT left as a high-valence hub of radiating sliver
/// triangles (the "starburst" the owner reported). Over this ~40-edge apex rim
/// a surviving single-hub fan would spike a vertex to valence ~40.
#[test]
fn fill_holes_interpolated_cap_has_no_sliver_fan_hub() {
    let (mesh, selection, _radius) = sphere_cap_with_apex_hole();
    let input_vertices = mesh.vertices.len();
    let result =
        fill_holes(&mesh, Some(&selection), MeshEditOptions::default()).expect("apex fill");

    let mut valence = vec![0usize; result.mesh.vertices.len()];
    let mut counted = std::collections::HashSet::new();
    for triangle in result.mesh.indices.chunks_exact(3) {
        let t: [usize; 3] = [
            triangle[0] as usize,
            triangle[1] as usize,
            triangle[2] as usize,
        ];
        for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            if counted.insert((a.min(b), a.max(b))) {
                valence[a] += 1;
                valence[b] += 1;
            }
        }
    }
    // Delaunay interior valence clusters around 6; a bounded ceiling proves no
    // fan hub survived. (A raw ear-clip fan would spike to ~rim length.)
    for (index, &deg) in valence.iter().enumerate().skip(input_vertices) {
        assert!(
            deg <= 12,
            "generated vertex {index} has valence {deg}: a sliver-fan hub survived"
        );
    }
}

#[test]
fn fill_holes_interpolated_cap_interpolates_rim_attributes() {
    let rim_color = [200, 100, 50, 255];
    let mesh = wavy_fan_mesh(rim_color);
    let input_vertices = mesh.vertices.len();
    let options = MeshEditOptions {
        protect_scan_border: false, // lone-fan fixture: its rim is no border
        ..MeshEditOptions::default()
    };
    let result = fill_holes(&mesh, None, options).expect("interpolated fill");

    assert!(result.mesh.vertices.len() > input_vertices);
    for vertex in &result.mesh.vertices[input_vertices..] {
        // All cap ancestors carry the rim color, so interpolation must too.
        assert_eq!(vertex.color, rim_color);
    }
}

/// A spherical cap (radius `r`) sampled on a theta/phi grid, with a round hole
/// left open at the apex and the outer ring flagged so a selection can leave it
/// open. The rim of the apex hole is a single planar ring — it carries no
/// curvature on its own, so this exercises the support-band curvature fit.
fn sphere_cap_with_apex_hole() -> (MeshEditBuffers, FaceSelection, f32) {
    let radius = 20.0_f32;
    let sectors = 40usize;
    let rings = 14usize;
    let hole_theta = 0.35_f32;
    let max_theta = 1.2_f32;

    let mut vertices: Vec<EditVertex> = Vec::new();
    let mut grid: Vec<Vec<usize>> = Vec::new();
    for ri in 0..=rings {
        let theta = hole_theta + (max_theta - hole_theta) * (ri as f32 / rings as f32);
        let mut row = Vec::new();
        for si in 0..sectors {
            let phi = std::f32::consts::TAU * (si as f32 / sectors as f32);
            let p = [
                radius * theta.sin() * phi.cos(),
                radius * theta.cos(),
                radius * theta.sin() * phi.sin(),
            ];
            row.push(vertices.len());
            vertices.push(v(p));
        }
        grid.push(row);
    }

    let mut indices = Vec::new();
    let mut mask = Vec::new();
    for ri in 0..rings {
        for si in 0..sectors {
            let s1 = (si + 1) % sectors;
            let a = grid[ri][si] as u32;
            let b = grid[ri][s1] as u32;
            let c = grid[ri + 1][s1] as u32;
            let d = grid[ri + 1][si] as u32;
            // Two triangles per quad, outward winding.
            indices.extend_from_slice(&[a, d, c]);
            indices.extend_from_slice(&[a, c, b]);
            // Leave the outermost ring of faces UNSELECTED so only the apex
            // hole is filled (the outer boundary stays open).
            let interior = ri + 1 < rings;
            mask.push(interior);
            mask.push(interior);
        }
    }

    let mesh = MeshEditBuffers {
        vertices,
        indices,
        topology: MeshTopology::TriangleMesh,
    };
    let selection = FaceSelection::new(mask);
    (mesh, selection, radius)
}

/// The cap must follow the surrounding curvature, not drape a flat disk. A flat
/// cap over this apex hole would sag ~1.1 mm below the sphere; the fitted cap
/// keeps every generated vertex on the sphere to a tight tolerance.
#[test]
fn fill_holes_interpolated_cap_follows_surrounding_curvature() {
    let (mesh, selection, radius) = sphere_cap_with_apex_hole();
    let input_vertices = mesh.vertices.len();
    let result =
        fill_holes(&mesh, Some(&selection), MeshEditOptions::default()).expect("apex fill");

    assert_eq!(result.report.filled_holes, 1, "only the apex hole fills");
    assert!(
        result.mesh.vertices.len() > input_vertices,
        "the interpolated cap must generate interior vertices"
    );

    let mut worst = 0.0_f32;
    for vertex in &result.mesh.vertices[input_vertices..] {
        let distance = glam::Vec3::from_array(vertex.position).length();
        worst = worst.max((distance - radius).abs());
    }
    assert!(
        worst < 0.3,
        "cap vertex strayed {worst:.3} mm from the sphere (a flat disk would sag ~1.1 mm)"
    );
}

/// The seam between cap and surrounding surface must be smooth: the dihedral
/// angle across every original rim edge stays well below a visible crease. This
/// is the measurable form of "closed with a smooth surface based on the
/// surrounding shape".
#[test]
fn fill_holes_interpolated_cap_seam_is_smooth() {
    let (mesh, selection, _radius) = sphere_cap_with_apex_hole();
    let rim = boundary_edges_undirected(&mesh.indices);
    let result =
        fill_holes(&mesh, Some(&selection), MeshEditOptions::default()).expect("apex fill");

    let mut edge_faces: std::collections::HashMap<(u32, u32), Vec<usize>> =
        std::collections::HashMap::new();
    for (ti, triangle) in result.mesh.indices.chunks_exact(3).enumerate() {
        for (a, b) in [
            (triangle[0], triangle[1]),
            (triangle[1], triangle[2]),
            (triangle[2], triangle[0]),
        ] {
            edge_faces.entry((a.min(b), a.max(b))).or_default().push(ti);
        }
    }

    let mut worst_deg = 0.0_f32;
    for (a, b) in rim {
        let key = (a.min(b), a.max(b));
        if let Some(faces) = edge_faces.get(&key) {
            if let [t0, t1] = faces.as_slice() {
                let n0 = face_normal(&result.mesh, *t0);
                let n1 = face_normal(&result.mesh, *t1);
                if n0.length_squared() > 1e-12 && n1.length_squared() > 1e-12 {
                    let cosine = n0.normalize().dot(n1.normalize()).clamp(-1.0, 1.0);
                    worst_deg = worst_deg.max(cosine.acos().to_degrees());
                }
            }
        }
    }
    assert!(
        worst_deg < 15.0,
        "seam dihedral {worst_deg:.1} deg is a visible crease"
    );
}

fn boundary_edges_undirected(indices: &[u32]) -> Vec<(u32, u32)> {
    let mut directed = std::collections::HashSet::new();
    for triangle in indices.chunks_exact(3) {
        for e in [
            (triangle[0], triangle[1]),
            (triangle[1], triangle[2]),
            (triangle[2], triangle[0]),
        ] {
            directed.insert(e);
        }
    }
    directed
        .iter()
        .copied()
        .filter(|&(a, b)| !directed.contains(&(b, a)))
        .collect()
}

fn face_normal(mesh: &MeshEditBuffers, triangle_index: usize) -> glam::Vec3 {
    let tri = &mesh.indices[triangle_index * 3..triangle_index * 3 + 3];
    let a = glam::Vec3::from_array(mesh.vertices[tri[0] as usize].position);
    let b = glam::Vec3::from_array(mesh.vertices[tri[1] as usize].position);
    let c = glam::Vec3::from_array(mesh.vertices[tri[2] as usize].position);
    (b - a).cross(c - a)
}

#[test]
fn fill_holes_skips_loops_larger_than_the_conservative_limit() {
    let mesh = bowl_mesh();
    let result = fill_holes(
        &mesh,
        None,
        MeshEditOptions {
            max_boundary_loop: 3,
            ..MeshEditOptions::default()
        },
    )
    .expect("skip large loop");

    assert_eq!(result.report.filled_holes, 0);
    assert_eq!(result.report.output_triangles, mesh.triangle_count());
    assert_eq!(
        result.report.warnings,
        vec![MeshEditWarning::DegenerateGeometry]
    );
}

#[test]
fn fill_holes_refuses_non_simple_self_intersecting_rim() {
    // A 4-vertex fan whose rim, in order 0->1->2->3, is an hourglass (the
    // edges 0-1 and 2-3 cross): the loop is non-simple, so no valid planar cap
    // exists. It is refused wholesale rather than emitted as a partial,
    // non-watertight patch, reported as skipped, and nothing is added.
    let mesh = MeshEditBuffers {
        vertices: vec![
            v([0.0, 0.0, 0.0]),
            v([1.0, 1.0, 0.0]),
            v([1.0, 0.0, 0.0]),
            v([0.0, 1.0, 0.0]),
            v([0.5, 0.5, -1.0]),
        ],
        indices: vec![4, 0, 1, 4, 1, 2, 4, 2, 3, 4, 3, 0],
        topology: MeshTopology::TriangleMesh,
    };
    let before_triangles = mesh.triangle_count();
    let result = fill_holes(&mesh, None, MeshEditOptions::default()).expect("non-simple rim");

    assert_eq!(result.report.filled_holes, 0);
    assert_eq!(result.report.output_triangles, before_triangles);
    assert_eq!(
        result.report.warnings,
        vec![MeshEditWarning::DegenerateGeometry]
    );
}

#[test]
fn fill_holes_splits_and_closes_rims_sharing_a_pinch_vertex() {
    // Bowtie: two triangles joined at a single vertex (2). The pinch is split
    // so the rims are walked independently (the dental-lab "closes random
    // holes" bug), but each rim here belongs to a LONE triangle whose only cap
    // is its own reverse twin — a zero-volume doubled sliver — so both fills
    // are honestly refused. Real adjacent pinched holes (surfaces, not lone
    // triangles) close: see holes_tests::adjacent_pinched_rims_both_close.
    let mesh = MeshEditBuffers {
        vertices: vec![
            v([0.0, 0.0, 0.0]),
            v([1.0, 0.0, 0.0]),
            v([0.0, 1.0, 0.0]),
            v([2.0, 0.0, 0.0]),
            v([2.0, 1.0, 0.0]),
        ],
        indices: vec![0, 1, 2, 2, 3, 4],
        topology: MeshTopology::TriangleMesh,
    };
    let before_triangles = mesh.triangle_count();
    let result = fill_holes(&mesh, None, MeshEditOptions::default()).expect("pinch fill");

    assert_eq!(result.report.filled_holes, 0);
    assert_eq!(result.report.output_triangles, before_triangles);
    assert_eq!(result.report.warnings.len(), 2);
}
