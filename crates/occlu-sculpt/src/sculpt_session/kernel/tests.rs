use super::*;

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
