use super::*;

fn grid_session(half_cells: usize, spacing: f64, spike_height: f64) -> SculptSession {
    let side = half_cells * 2 + 1;
    let mut verts = Vec::with_capacity(side * side * 3);
    for y in 0..side {
        for x in 0..side {
            let px = (x as f64 - half_cells as f64) * spacing;
            let py = (y as f64 - half_cells as f64) * spacing;
            let height = if x == half_cells && y == half_cells {
                spike_height
            } else {
                0.0
            };
            verts.extend_from_slice(&[px as f32, py as f32, height as f32]);
        }
    }
    let mut tris = Vec::with_capacity((side - 1) * (side - 1) * 6);
    let index = |x: usize, y: usize| (y * side + x) as u32;
    for y in 0..side - 1 {
        for x in 0..side - 1 {
            let (a, b, c, d) = (
                index(x, y),
                index(x + 1, y),
                index(x, y + 1),
                index(x + 1, y + 1),
            );
            tris.extend_from_slice(&[a, b, d, a, d, c]);
        }
    }
    SculptSession::new(verts, tris)
}

fn grid_vertex(half_cells: usize, x: usize, y: usize) -> u32 {
    ((y + half_cells) * (half_cells * 2 + 1) + x + half_cells) as u32
}

fn cube_crease_session() -> SculptSession {
    let faces = [
        [
            [-1.0, -1.0, 1.0],
            [1.0, -1.0, 1.0],
            [1.0, 1.0, 1.0],
            [-1.0, 1.0, 1.0],
        ],
        [
            [-1.0, 1.0, -1.0],
            [-1.0, 1.0, 1.0],
            [1.0, 1.0, 1.0],
            [1.0, 1.0, -1.0],
        ],
    ];
    let mut verts = Vec::with_capacity(24);
    let mut tris = Vec::with_capacity(12);
    for (face_index, face) in faces.into_iter().enumerate() {
        verts.extend(face.into_iter().flatten());
        let base = (face_index * 4) as u32;
        tris.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    SculptSession::new(verts, tris)
}

fn centered_dab(radius: f64, mode: BrushMode, strength: f64) -> Dab {
    Dab {
        center: DVec3::ZERO,
        radius,
        strength,
        view: DVec3::new(0.0, 0.0, -1.0),
        mode,
    }
}

fn center_curvature(session: &SculptSession, center_vertex: u32) -> f64 {
    let group = session.topology.group_of(center_vertex);
    let neighbors = session.topology.neighbors(group);
    let neighbor_mean = neighbors
        .iter()
        .map(|&neighbor| session.group_v(neighbor).z)
        .sum::<f64>()
        / neighbors.len() as f64;
    session.group_v(group).z - neighbor_mean
}

fn select_all(session: &mut SculptSession) {
    session.region_points = (0..session.topology.group_count() as u32)
        .filter(|&group| session.group_is_live(group))
        .map(|group| SurfacePoint {
            group,
            distance: 0.0,
        })
        .collect();
    session.snapshot_region();
}

/// Check the acceleration rows against the authoritative raw index buffer.
fn assert_rows(session: &SculptSession) {
    use std::collections::BTreeSet;
    let faces: Vec<[u32; 3]> = session
        .tris
        .as_chunks::<3>()
        .0
        .iter()
        .map(|face| (*face).map(|v| session.topology.group_of(v)))
        .collect();
    assert_eq!(session.live_tris as usize, faces.len());
    for (id, face) in faces.iter().enumerate() {
        assert_eq!(session.topology.triangle(id as u32), Some(*face));
    }
    for group in 0..session.topology.group_count() as u32 {
        let incident: BTreeSet<u32> = faces
            .iter()
            .enumerate()
            .filter(|(_, face)| face.contains(&group))
            .map(|(id, _)| id as u32)
            .collect();
        assert_eq!(
            session
                .topology
                .incident_triangles(group)
                .iter()
                .copied()
                .collect::<BTreeSet<_>>(),
            incident
        );
        let neighbors: BTreeSet<u32> = incident
            .iter()
            .flat_map(|&id| faces[id as usize])
            .filter(|&corner| corner != group)
            .collect();
        assert_eq!(
            session
                .topology
                .neighbors(group)
                .iter()
                .copied()
                .collect::<BTreeSet<_>>(),
            neighbors
        );
        // Include absent edges: stale rows must be empty after a collapse/flip.
        for other in 0..session.topology.group_count() as u32 {
            if other == group {
                continue;
            }
            let expected: BTreeSet<u32> = incident
                .iter()
                .copied()
                .filter(|&id| faces[id as usize].contains(&other))
                .collect();
            assert_eq!(
                session
                    .topology
                    .edge_triangles(group, other)
                    .iter()
                    .copied()
                    .collect::<BTreeSet<_>>(),
                expected,
                "edge {group}-{other}"
            );
        }
    }
}

#[test]
fn dab_snapshot_survives_scratch_traversal_and_relocates_raycast() {
    let mut session = SculptSession::new(
        vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
        vec![0, 1, 2],
    );
    session.start_stroke();
    select_all(&mut session);
    for group in 0..3 {
        let before = session.group_v(group);
        session.write_group_position(group, before + DVec3::new(10.0, 0.0, 0.0));
    }
    let _ = session.collect_normal_scope(&[0, 1, 2]);
    assert_eq!(session.pre_group(0), DVec3::ZERO);
    assert!(!session.after_dab_maintenance().is_empty());
    assert!(session
        .raycast(DVec3::new(10.2, 0.2, 1.0), DVec3::new(0.0, 0.0, -1.0))
        .is_some());
    assert!(session
        .raycast(DVec3::new(0.2, 0.2, 1.0), DVec3::new(0.0, 0.0, -1.0))
        .is_none());
}

#[test]
fn flip_uses_group_ids_then_journals_raw_vertices() {
    // The duplicate unused corners offset every live group from its raw id.
    let verts = vec![
        10.0, 10.0, 0.0, 10.0, 10.0, 0.0, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 2.0, 1.0, 0.0, 0.0, 2.0,
        0.0,
    ];
    let tris = vec![2, 3, 5, 3, 4, 5];
    let mut session = SculptSession::new(verts.clone(), tris.clone());
    session.start_stroke();
    select_all(&mut session);
    let tolerance = SculptSession::remesh_tolerance_mm(1.0);
    let mut journal = std::mem::take(&mut session.topo_journal);
    assert!(session.try_flip_edge(2, 4, 4.0, tolerance, &mut journal));
    session.topo_journal = journal;
    assert_rows(&session);
    let after = session.tris.clone();
    assert_ne!(after, tris);
    let record = session.end_stroke();
    assert_eq!(session.tris, after, "release must not edit geometry");
    assert_eq!(
        record.journal.encoded_size_words(),
        record.journal.encode_u32().len() + record.journal.encode_f32().len()
    );
    session
        .restore_topo(&record.indices, &record.before, false, &record.journal)
        .unwrap();
    assert_eq!(session.tris, tris);
    assert_eq!(session.verts, verts);
    assert_rows(&session);
    session
        .restore_topo(&record.indices, &record.after, true, &record.journal)
        .unwrap();
    assert_eq!(session.tris, after);
    assert_rows(&session);
}

#[test]
fn local_surface_accepts_a_flat_retriangulation_but_not_a_cut_across_a_ridge() {
    use super::remesh::LocalSurface;
    let (a, b) = (DVec3::new(0.0, 0.0, 0.0), DVec3::new(1.0, 1.0, 0.0));
    let (c, d) = (DVec3::new(1.0, 0.0, 0.0), DVec3::new(0.0, 1.0, 0.0));
    let tolerance = 0.01;
    // A flat square read one way covers the square read the other way.
    let mut flat = LocalSurface::new();
    assert!(flat.push(0, [a, c, b]));
    assert!(flat.push(1, [a, b, d]));
    assert!(flat.covers([c, d, a], tolerance, Some(0)));
    assert!(flat.covers([c, b, d], tolerance, Some(1)));
    // Raise the diagonal into a ridge: the other diagonal cuts under it by
    // half the ridge height at its midpoint, far past the tolerance.
    let (ridge_a, ridge_b) = (
        (a + DVec3::new(0.0, 0.0, 0.1)),
        (b + DVec3::new(0.0, 0.0, 0.1)),
    );
    let mut ridge = LocalSurface::new();
    assert!(ridge.push(0, [ridge_a, c, ridge_b]));
    assert!(ridge.push(1, [ridge_a, ridge_b, d]));
    assert!(!ridge.covers([c, d, ridge_a], tolerance, Some(0)));
    let (landing, distance) = ridge.nearest(DVec3::new(0.5, 0.5, 0.3)).unwrap();
    assert!((distance - 0.2).abs() < 1e-12);
    assert!((landing.z - 0.1).abs() < 1e-12);
}

#[test]
fn split_and_collapse_keep_rows_and_exact_history() {
    let mut verts = Vec::new();
    let mut tris = Vec::new();
    for y in 0..4 {
        for x in 0..4 {
            verts.extend_from_slice(&[x as f32, y as f32, 0.0]);
        }
    }
    for y in 0..3 {
        for x in 0..3 {
            let a = y * 4 + x;
            tris.extend_from_slice(&[a, a + 1, a + 5, a, a + 5, a + 4]);
        }
    }
    let mut session = SculptSession::new(verts.clone(), tris.clone());
    session.start_stroke();
    select_all(&mut session);
    let dab = Dab {
        center: DVec3::new(1.5, 1.5, 0.0),
        radius: 2.4,
        strength: 0.0,
        view: DVec3::new(0.0, 0.0, -1.0),
        mode: BrushMode::Smooth,
    };
    let policy = RemeshPolicy::standard();
    session.isotropic_cycle(&dab, &policy, 0.4);
    assert!(
        !session.topo_journal.added_verts.is_empty(),
        "exercise a real split"
    );
    assert!(session.dab_topo_ops <= policy.max_operations_per_dab);
    assert_rows(&session);
    // A larger brush must allow removal of surplus samples in the same session.
    session.dab_topo_ops = 0;
    select_all(&mut session);
    let tolerance = SculptSession::remesh_tolerance_mm(1.5);
    let mut journal = std::mem::take(&mut session.topo_journal);
    let region = session.region_points.clone();
    session.collapse_footprint_inner(
        &region,
        &policy,
        1.5,
        session.live_tris,
        tolerance,
        &mut journal,
    );
    assert!(!journal.collapsed.is_empty(), "exercise a real collapse");
    session.topo_journal = journal;
    assert_rows(&session);
    let after = (
        session.verts.clone(),
        session.tris.clone(),
        session.face_origins(),
    );
    let record = session.end_stroke();
    assert_eq!(
        record.journal.encoded_size_words(),
        record.journal.encode_u32().len() + record.journal.encode_f32().len()
    );
    session
        .restore_topo(&record.indices, &record.before, false, &record.journal)
        .unwrap();
    assert_eq!((&session.verts, &session.tris), (&verts, &tris));
    assert_rows(&session);
    session
        .restore_topo(&record.indices, &record.after, true, &record.journal)
        .unwrap();
    assert_eq!(
        (
            session.verts.clone(),
            session.tris.clone(),
            session.face_origins()
        ),
        after
    );
    assert_rows(&session);
}

#[test]
fn motion_limits_preserve_identity_and_requested_direction_after_compression() {
    let mut session = SculptSession::new(
        vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
        vec![0, 1, 2],
    );
    session.write_group_position(1, DVec3::new(0.1, 0.0, 0.0));
    let here = session.group_v(0);
    assert_eq!(session.clamp_step_at(0, here, here), here);
    let request = DVec3::new(0.2, 0.0, 0.0);
    let limited = session.clamp_step_at(0, here, request);
    assert!(limited.x > 0.0 && limited.x < 0.1);
    assert_eq!(limited.y, 0.0);
    assert_eq!(limited.z, 0.0);
    // A tangential request cannot acquire an outward correction from old edges.
    let sideways = DVec3::new(0.0, 0.02, 0.0);
    assert_eq!(session.clamp_step_at(0, here, sideways), sideways);
}

#[test]
fn smooth_keeps_an_irregular_planar_fan_in_place() {
    let verts = vec![
        0.2, 0.1, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, -1.0, 0.0, 0.0, 0.0, -1.0, 0.0,
    ];
    let mut session = SculptSession::new(verts.clone(), vec![0, 1, 2, 0, 2, 3, 0, 3, 4, 0, 4, 1]);
    session.start_stroke();
    select_all(&mut session);
    let region = session.region_points.clone();
    session.dab_smooth(
        &Dab {
            center: DVec3::ZERO,
            radius: 2.0,
            strength: 0.5,
            view: DVec3::new(0.0, 0.0, -1.0),
            mode: BrushMode::Smooth,
        },
        &region,
        1.0,
    );
    assert!(session.live_kin.weighted > 0, "exercise the shape solve");
    assert_eq!(session.verts, verts, "spacing changes belong to remesh");
}

#[test]
fn split_inherits_material_depth_across_strokes_and_history() {
    let mut verts = Vec::new();
    let mut tris = Vec::new();
    for y in 0..4 {
        for x in 0..4 {
            verts.extend_from_slice(&[x as f32, y as f32, 0.0]);
        }
    }
    for y in 0..3 {
        for x in 0..3 {
            let a = y * 4 + x;
            tris.extend_from_slice(&[a, a + 1, a + 5, a, a + 5, a + 4]);
        }
    }
    let mut session = SculptSession::new(verts, tris);
    for group in 0..16 {
        let point = session.group_v(group) + DVec3::new(0.0, 0.0, -0.6);
        session.write_group_position(group, point);
    }
    let dab = Dab {
        center: DVec3::new(1.5, 1.5, -0.6),
        radius: 2.4,
        strength: 0.2,
        view: DVec3::new(0.0, 0.0, -1.0),
        mode: BrushMode::Smooth,
    };
    for _ in 0..4 {
        session.start_stroke();
        select_all(&mut session);
        session.dab_topo_ops = 0;
        session.isotropic_cycle(&dab, &RemeshPolicy::standard(), 0.4);
        assert!(
            !session.topo_journal.added_verts.is_empty(),
            "split in each new stroke"
        );
        assert_rows(&session);
        for group in 0..session.topology.group_count() as u32 {
            assert_eq!(session.reference_group_v(group).z, 0.0);
            assert_eq!(session.reference_group_n(group), DVec3::new(0.0, 0.0, 1.0));
        }
        let reference = session.reference_verts.clone();
        let reference_normals = session.reference_normals.clone();
        let record = session.end_stroke();
        session
            .restore_topo(&record.indices, &record.before, false, &record.journal)
            .unwrap();
        session
            .restore_topo(&record.indices, &record.after, true, &record.journal)
            .unwrap();
        assert_eq!(session.reference_verts, reference);
        assert_eq!(session.reference_normals, reference_normals);
        assert_rows(&session);
    }
}

#[test]
fn add_and_remove_move_the_surface_by_the_brush_dose() {
    let center = grid_vertex(8, 0, 0);
    let dab = centered_dab(2.0, BrushMode::Deposit, 1.0);
    let mut raised = grid_session(8, 0.5, 0.0);
    assert!(!raised.dab(&dab).is_empty());
    let lift = raised.group_v(raised.topology.group_of(center)).z;
    assert!((0.04..=0.14).contains(&lift), "the center lift is {lift}");

    let mut lowered = grid_session(8, 0.5, 0.0);
    let remove = Dab {
        mode: BrushMode::Erode,
        ..dab
    };
    assert!(!lowered.dab(&remove).is_empty());
    let cut = lowered.group_v(lowered.topology.group_of(center)).z;
    assert!((-0.14..=-0.04).contains(&cut), "the center cut is {cut}");
}

#[test]
fn smooth_reduces_the_curvature_of_a_synthetic_spike() {
    let center = grid_vertex(6, 0, 0);
    let mut session = grid_session(6, 0.5, 2.0);
    let before = center_curvature(&session, center).abs();
    let dab = centered_dab(2.5, BrushMode::Smooth, 1.0);
    for _ in 0..8 {
        assert!(!session.dab(&dab).is_empty());
    }
    let after = center_curvature(&session, center).abs();
    assert!(
        after < before * 0.75,
        "curvature changed from {before} to {after}"
    );
}

#[test]
fn knife_displacement_follows_its_stroke_axis() {
    let mut session = grid_session(10, 0.5, 0.0);
    session.set_brush_tip(TipStamp::Knife);
    session.set_dab_axis(Some(DVec3::X));
    let changed = session.dab(&centered_dab(4.0, BrushMode::Deposit, 1.0));
    assert!(!changed.is_empty());
    let along = session
        .group_v(session.topology.group_of(grid_vertex(10, 4, 0)))
        .z;
    let across = session
        .group_v(session.topology.group_of(grid_vertex(10, 0, 4)))
        .z;
    assert!(
        along > across + 0.04,
        "axis lift={along}, cross lift={across}"
    );
}

#[test]
fn cylinder_tip_makes_a_flat_deposit_plateau() {
    let mut session = grid_session(16, 0.25, 0.0);
    session.set_brush_tip(TipStamp::Cylinder);
    assert!(!session
        .dab(&centered_dab(3.5, BrushMode::Deposit, 1.0))
        .is_empty());
    let center = session
        .group_v(session.topology.group_of(grid_vertex(16, 0, 0)))
        .z;
    let plateau = session
        .group_v(session.topology.group_of(grid_vertex(16, 10, 0)))
        .z;
    let rim = session
        .group_v(session.topology.group_of(grid_vertex(16, 13, 0)))
        .z;
    assert!(
        (center - plateau).abs() < 0.025,
        "center={center}, plateau={plateau}"
    );
    assert!(plateau > rim + 0.02, "plateau={plateau}, rim={rim}");
}

#[test]
fn sculpt_dab_preserves_split_shading_normals_on_a_cube_crease() {
    use glam::Vec3;

    let mut session = cube_crease_session();
    let top_corner = 2;
    let side_corner = 6;
    assert_eq!(
        session.topology.group_of(top_corner),
        session.topology.group_of(side_corner),
        "the two face corners share one welded sculpt position"
    );
    session.start_stroke();
    let changed = session.dab(&Dab {
        center: DVec3::new(0.0, 0.0, 1.0),
        radius: 2.5,
        strength: 0.8,
        view: DVec3::new(0.0, 0.0, -1.0),
        mode: BrushMode::Deposit,
    });
    assert!(!changed.is_empty(), "the cube crease accepts the dab");

    let normals = session.normals();
    let top = Vec3::from_slice(&normals[top_corner as usize * 3..top_corner as usize * 3 + 3]);
    let side = Vec3::from_slice(&normals[side_corner as usize * 3..side_corner as usize * 3 + 3]);
    assert!(top.dot(Vec3::Z) > 0.7, "top normal was {top:?}");
    assert!(side.dot(Vec3::Y) > 0.7, "side normal was {side:?}");
    assert!(
        top.dot(side) < 0.6,
        "crease normals were {top:?} and {side:?}"
    );
    let record = session.end_stroke();
    let top_slot = record
        .normal_indices
        .iter()
        .position(|&vertex| vertex == top_corner)
        .expect("the moved top corner has a display-normal record");
    assert_eq!(
        &record.normal_values[top_slot * 3..top_slot * 3 + 3],
        &top.to_array()
    );
}

fn run_long_remesh_stroke(session: &mut SculptSession) {
    session.start_stroke();
    let dab = centered_dab(12.0, BrushMode::Smooth, 0.5);
    for _ in 0..12 {
        let _ = session.dab(&dab);
    }
}

fn assert_live_surface_invariants(session: &SculptSession, target: f64) {
    use std::collections::BTreeMap;
    let mut edges: BTreeMap<(u32, u32), (usize, i32)> = BTreeMap::new();
    let mut min_edge = f64::INFINITY;
    let mut max_edge = 0.0_f64;
    for face in session.faces().as_chunks::<3>().0 {
        let [a, b, c] = *face;
        assert!(a != b && b != c && c != a);
        let points = [
            session.group_v(session.topology.group_of(a)),
            session.group_v(session.topology.group_of(b)),
            session.group_v(session.topology.group_of(c)),
        ];
        let normal = (points[1] - points[0]).cross(points[2] - points[0]);
        assert!(normal.length() > 1e-8, "a live triangle is degenerate");
        assert!(normal.z > 0.0, "a live triangle is inverted");
        for (from, to) in [(a, b), (b, c), (c, a)] {
            let (key, direction) = if from < to {
                ((from, to), 1)
            } else {
                ((to, from), -1)
            };
            let edge = (session.group_v(session.topology.group_of(from))
                - session.group_v(session.topology.group_of(to)))
            .length();
            min_edge = min_edge.min(edge);
            max_edge = max_edge.max(edge);
            let incidence = edges.entry(key).or_default();
            incidence.0 += 1;
            incidence.1 += direction;
        }
    }
    assert!(edges
        .values()
        .all(|&(count, direction)| count <= 2 && (count == 1 || direction == 0)));
    let policy = RemeshPolicy::standard();
    assert!(
        min_edge >= target * policy.collapse_hysteresis * 0.3,
        "short edge {min_edge} fell below the remesh band"
    );
    assert!(
        max_edge <= target * policy.split_hysteresis * 1.5,
        "long edge {max_edge} exceeded the remesh band"
    );
}

#[test]
fn long_remeshing_stroke_stays_manifold_and_restores_exact_history() {
    let mut session = grid_session(2, 4.0, 0.0);
    let before_verts = session.verts.clone();
    let before_tris = session.faces().to_vec();
    run_long_remesh_stroke(&mut session);
    assert!(session.topology_revision() > 0, "the stroke must remesh");
    assert_live_surface_invariants(&session, 2.0);
    let after_verts = session.verts.clone();
    let after_tris = session.faces().to_vec();
    let record = session.end_stroke();
    assert!(session
        .restore_topo(&record.indices, &record.before, false, &record.journal)
        .is_some());
    assert_eq!(session.verts, before_verts);
    assert_eq!(session.faces(), before_tris);
    assert!(session
        .restore_topo(&record.indices, &record.after, true, &record.journal)
        .is_some());
    assert_eq!(session.verts, after_verts);
    assert_eq!(session.faces(), after_tris);
}

#[test]
fn identical_remeshing_strokes_are_bit_deterministic() {
    let mut first = grid_session(2, 4.0, 0.0);
    let mut second = grid_session(2, 4.0, 0.0);
    run_long_remesh_stroke(&mut first);
    run_long_remesh_stroke(&mut second);
    assert_eq!(first.verts, second.verts);
    assert_eq!(first.faces(), second.faces());
    assert_eq!(first.normals(), second.normals());
    assert_eq!(first.topology_revision(), second.topology_revision());
}

#[cfg(feature = "parallel")]
#[test]
fn large_layer_commit_is_bit_identical_across_worker_counts() {
    fn commit(pool: &rayon::ThreadPool) -> (Vec<f32>, Vec<f32>) {
        let mut session = grid_session(46, 0.14, 0.8);
        session.start_stroke();
        select_all(&mut session);
        let before_normals = session.normals().to_vec();
        let proposals: Vec<(u32, DVec3)> = (0..session.topology.group_count() as u32)
            .map(|group| {
                let here = session.group_v(group);
                let normal = session.group_n(group).normalize_or_zero();
                (group, here + (normal * 0.001))
            })
            .collect();
        let groups: Vec<u32> = (0..session.topology.group_count() as u32).collect();
        pool.install(|| {
            session.commit_even_layer(&proposals, BrushMode::Smooth);
            let scope = session.collect_normal_scope(&groups);
            session.refresh_scope_normals(&scope);
        });
        let normals = session.normals().to_vec();
        assert_ne!(normals, before_normals);
        (session.verts, normals)
    }

    let one_worker = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .expect("one-worker pool builds");
    let four_workers = rayon::ThreadPoolBuilder::new()
        .num_threads(4)
        .build()
        .expect("four-worker pool builds");
    let one = commit(&one_worker);
    let four = commit(&four_workers);
    assert_ne!(one.0, grid_session(46, 0.14, 0.8).verts);
    assert_eq!(one, four);
}

#[test]
fn non_finite_dab_input_is_refused_without_geometry_changes() {
    let mut session = grid_session(4, 1.0, 0.0);
    let original_verts = session.verts.clone();
    let original_faces = session.faces().to_vec();
    let base = centered_dab(2.0, BrushMode::Deposit, 1.0);
    for invalid in [
        Dab {
            center: DVec3::splat(f64::NAN),
            ..base
        },
        Dab {
            view: DVec3::new(f64::INFINITY, 0.0, 0.0),
            ..base
        },
        Dab {
            radius: f64::INFINITY,
            ..base
        },
        Dab {
            strength: f64::NAN,
            ..base
        },
    ] {
        assert!(session.dab(&invalid).is_empty());
        assert_eq!(session.verts, original_verts);
        assert_eq!(session.faces(), original_faces);
    }
}
