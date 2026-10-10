use super::*;

fn grid_session(half_cells: usize, spacing: f64, spike_height: f64) -> SculptSession {
    grid_session_with_offset_spike(half_cells, spacing, spike_height, 0)
}

fn grid_session_with_offset_spike(
    half_cells: usize,
    spacing: f64,
    spike_height: f64,
    spike_offset_x: isize,
) -> SculptSession {
    let side = half_cells * 2 + 1;
    let spike_x = half_cells as isize + spike_offset_x;
    let mut verts = Vec::with_capacity(side * side * 3);
    for y in 0..side {
        for x in 0..side {
            let px = (x as f64 - half_cells as f64) * spacing;
            let py = (y as f64 - half_cells as f64) * spacing;
            let height = if x as isize == spike_x && y == half_cells {
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
    SculptSession::new(verts, tris).expect("valid mesh fixture")
}

fn irregular_sheet_session() -> SculptSession {
    let side = 17;
    let spacing = 0.8;
    let mut verts = Vec::with_capacity(side * side * 3);
    for y in 0..side {
        for x in 0..side {
            let px = (x as f64 - (side as f64 - 1.0) * 0.5) * spacing;
            let py = (y as f64 - (side as f64 - 1.0) * 0.5) * spacing;
            let height = 0.18 * (0.9 * px).sin() * (0.7 * py).cos() + 0.01 * px * py;
            verts.extend_from_slice(&[px as f32, py as f32, height as f32]);
        }
    }
    let index = |x: usize, y: usize| (y * side + x) as u32;
    let mut tris = Vec::with_capacity((side - 1) * (side - 1) * 6);
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
    SculptSession::new(verts, tris).expect("valid mesh fixture")
}

fn assert_live_normal_buffers_match(
    session: &SculptSession,
    expected_brush: &[f32],
    expected_display: &[f32],
    context: &str,
) {
    let mut live = vec![false; session.vertex_count()];
    for &vertex in session.tris.iter().take(session.live_tris as usize * 3) {
        live[vertex as usize] = true;
    }
    for (vertex, is_live) in live.into_iter().enumerate() {
        if !is_live {
            continue;
        }
        let offset = vertex * 3;
        assert_eq!(
            &expected_brush[offset..offset + 3],
            &session.brush_normals[offset..offset + 3],
            "{context}: brush normal at live vertex {vertex}"
        );
        assert_eq!(
            &expected_display[offset..offset + 3],
            &session.display_normals[offset..offset + 3],
            "{context}: display normal at live vertex {vertex}"
        );
    }
}

fn tilted_grid_session(half_cells: usize, spacing: f64, slope: f64) -> SculptSession {
    let side = half_cells * 2 + 1;
    let mut verts = Vec::with_capacity(side * side * 3);
    for y in 0..side {
        for x in 0..side {
            let px = (x as f64 - half_cells as f64) * spacing;
            let py = (y as f64 - half_cells as f64) * spacing;
            verts.extend_from_slice(&[px as f32, py as f32, (px * slope) as f32]);
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
    SculptSession::new(verts, tris).expect("valid mesh fixture")
}

fn closed_slab_session(half_cells: usize, spacing: f64, thickness: f64) -> SculptSession {
    let side = half_cells * 2 + 1;
    let mut verts = Vec::with_capacity(2 * side * side * 3);
    for layer in 0..2 {
        let z = if layer == 0 { -thickness } else { 0.0 };
        for y in 0..side {
            for x in 0..side {
                let px = (x as f64 - half_cells as f64) * spacing;
                let py = (y as f64 - half_cells as f64) * spacing;
                verts.extend_from_slice(&[px as f32, py as f32, z as f32]);
            }
        }
    }
    let index =
        |layer: usize, x: usize, y: usize| -> u32 { (layer * side * side + y * side + x) as u32 };
    let mut tris = Vec::new();
    for y in 0..side - 1 {
        for x in 0..side - 1 {
            let (ba, bb, bc, bd) = (
                index(0, x, y),
                index(0, x + 1, y),
                index(0, x, y + 1),
                index(0, x + 1, y + 1),
            );
            let (ta, tb, tc, td) = (
                index(1, x, y),
                index(1, x + 1, y),
                index(1, x, y + 1),
                index(1, x + 1, y + 1),
            );
            // Top faces point out of +Z; bottom faces out of -Z.
            tris.extend_from_slice(&[ta, tb, td, ta, td, tc]);
            tris.extend_from_slice(&[ba, bd, bb, ba, bc, bd]);
        }
    }
    for x in 0..side - 1 {
        // y-min side points toward -Y; y-max points toward +Y.
        let (a, b, c, d) = (
            index(0, x, 0),
            index(0, x + 1, 0),
            index(1, x + 1, 0),
            index(1, x, 0),
        );
        tris.extend_from_slice(&[a, b, c, a, c, d]);
        let (a, b, c, d) = (
            index(0, x + 1, side - 1),
            index(0, x, side - 1),
            index(1, x, side - 1),
            index(1, x + 1, side - 1),
        );
        tris.extend_from_slice(&[a, b, c, a, c, d]);
    }
    for y in 0..side - 1 {
        // x-min side points toward -X; x-max points toward +X.
        let (a, b, c, d) = (
            index(0, 0, y + 1),
            index(0, 0, y),
            index(1, 0, y),
            index(1, 0, y + 1),
        );
        tris.extend_from_slice(&[a, b, c, a, c, d]);
        let (a, b, c, d) = (
            index(0, side - 1, y),
            index(0, side - 1, y + 1),
            index(1, side - 1, y + 1),
            index(1, side - 1, y),
        );
        tris.extend_from_slice(&[a, b, c, a, c, d]);
    }
    SculptSession::new(verts, tris).expect("valid mesh fixture")
}

fn grid_vertex(half_cells: usize, x: usize, y: usize) -> u32 {
    ((y + half_cells) * (half_cells * 2 + 1) + x + half_cells) as u32
}

#[test]
fn deposit_geometry_is_identical_from_normal_and_grazing_camera_angles() {
    fn deposit(view: DVec3) -> (Vec<f32>, Vec<u32>) {
        let mut session = grid_session(3, 0.5, 0.0);
        session.start_stroke();
        let _ = session.dab(&Dab {
            center: DVec3::ZERO,
            radius: 1.5,
            strength: 0.8,
            view: view.normalize_or_zero(),
            mode: BrushMode::Deposit,
        });
        (session.verts, session.tris)
    }

    let normal_view = deposit(DVec3::new(0.0, 0.0, -1.0));
    let grazing_view = deposit(DVec3::new(0.8, 0.0, -0.6));
    assert_eq!(normal_view.0, grazing_view.0);
    assert_eq!(normal_view.1, grazing_view.1);
}

#[test]
fn sheet_weight_keeps_side_surface_at_a_grazing_view() {
    let mut session = cube_crease_session();
    session.start_stroke();
    let dab = Dab {
        center: DVec3::new(0.0, 0.0, 1.0),
        radius: 2.5,
        strength: 0.8,
        view: DVec3::new(0.0, 0.98, -0.2).normalize_or_zero(),
        mode: BrushMode::Deposit,
    };
    assert!(session.prepare_dab(&dab));
    let region = std::mem::take(&mut session.region_points);
    session.assign_sheet_axes(&dab, &region);
    let side_group = session.topology.group_of(4);
    let side_point = region
        .iter()
        .find(|point| point.group == side_group)
        .copied()
        .expect("the brush footprint reaches the orthogonal side");
    assert!(session.sheet_share(side_group) > 0.0);
    assert!(session.weight(side_point, &dab) > 0.0);
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
    SculptSession::new(verts, tris).expect("valid mesh fixture")
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
    )
    .expect("valid mesh fixture");
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
    let mut session = SculptSession::new(verts.clone(), tris.clone()).expect("valid mesh fixture");
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
    let mut session = SculptSession::new(verts.clone(), tris.clone()).expect("valid mesh fixture");
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
    let base_groups = session.topology.base_group_count();
    assert!(
        (base_groups..session.topology.group_count() as u32).any(|group| session
            .topology
            .incident_triangles_or_empty(group)
            .is_empty()),
        "the stroke leaves an appended group without live faces"
    );
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
    )
    .expect("valid mesh fixture");
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
    let mut session = SculptSession::new(verts.clone(), vec![0, 1, 2, 0, 2, 3, 0, 3, 4, 0, 4, 1])
        .expect("valid mesh fixture");
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
    let mut session = SculptSession::new(verts, tris).expect("valid mesh fixture");
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
fn add_and_remove_keep_the_taubin_field_within_one_full_brush_dose() {
    let center = grid_vertex(8, 0, 0);
    let dab = centered_dab(2.0, BrushMode::Deposit, 1.0);
    let mut raised = grid_session(8, 0.5, 0.0);
    assert!(!raised.dab(&dab).is_empty());
    let lift = raised.group_v(raised.topology.group_of(center)).z;
    let expected = layer_depth(dab.radius, dab.strength);
    assert!(
        (f64::from(raised.live_kin.amplitude) - expected).abs() < 1e-7,
        "the Add field amplitude should be one pass dose {expected}"
    );
    assert!(
        lift > 0.0 && lift <= expected + 1e-6,
        "Taubin may redistribute the center lift {lift}, but must cap it at {expected}"
    );

    let mut lowered = grid_session(8, 0.5, 0.0);
    let remove = Dab {
        mode: BrushMode::Erode,
        ..dab
    };
    assert!(!lowered.dab(&remove).is_empty());
    let cut = lowered.group_v(lowered.topology.group_of(center)).z;
    assert!(
        (f64::from(lowered.live_kin.amplitude) - expected).abs() < 1e-7,
        "the Remove field amplitude should be one pass dose {expected}"
    );
    assert!(
        cut < 0.0 && cut >= -expected - 1e-6,
        "Taubin may redistribute the center cut {cut}, but must cap it at {}",
        -expected
    );
}

#[test]
fn add_uses_the_sheet_axis_from_a_tilted_view() {
    let mut session = grid_session(8, 0.5, 0.0);
    let view = DVec3::new(0.6, 0.0, -0.8);
    let dab = Dab {
        view,
        ..centered_dab(2.0, BrushMode::Deposit, 1.0)
    };
    let center = grid_vertex(8, 0, 0);
    let before = session.v(center);
    let moved = session.dab(&dab);
    let delta = session.group_v(session.topology.group_of(center)) - before;
    let expected = layer_depth(dab.radius, dab.strength);
    assert!(!moved.is_empty());
    assert!(
        delta.x.abs() < 1e-6 && delta.z > 0.0 && delta.z <= expected + 1e-6,
        "surface-normal move was {delta:?}"
    );
    assert!(
        delta.normalize().dot(DVec3::Z) > 0.99,
        "the lift was not parallel to the sheet axis: {delta:?}"
    );
}

#[test]
fn held_add_keeps_its_stroke_axis_through_remesh_and_history() {
    let mut session = tilted_grid_session(4, 0.75, 0.4);
    let center_vertex = grid_vertex(4, 0, 0);
    let center_group = session.topology.group_of(center_vertex);
    let stroke_axis = session.group_n(center_group).normalize_or_zero();
    assert!(stroke_axis.length_squared() > 0.99);
    session.start_stroke();

    let radius = 2.4;
    let mut first_children = Vec::new();
    for step in 0..3 {
        session.set_dab_elapsed_ms(DWELL_FULL_DOSE_MS);
        let center = session.group_v(center_group);
        let view = if step == 1 {
            // Still aimed through the same surface point, but at a different
            // camera angle while the held stroke remains open.
            (-stroke_axis + DVec3::Y * 0.35).normalize_or_zero()
        } else {
            -stroke_axis
        };
        let before = center;
        let moved = session.dab(&Dab {
            center,
            radius,
            strength: 0.8,
            view,
            mode: BrushMode::Deposit,
        });
        assert!(
            !moved.is_empty(),
            "held deposit {step} must move the surface"
        );
        let delta = session.group_v(center_group) - before;
        assert!(
            delta.dot(stroke_axis) > 0.01,
            "held deposit {step} lost its original sheet axis: {delta:?}"
        );
        assert!(
            (delta - stroke_axis * delta.dot(stroke_axis)).length() < 1e-4,
            "held deposit {step} slid across the sheet: {delta:?}"
        );
        if step == 0 {
            first_children.extend_from_slice(session.dab_added_parents());
        }
    }

    assert!(
        !first_children.is_empty(),
        "the held stroke must exercise live remesh splits"
    );
    let epoch = session.stroke_epoch;
    let mut checked_inheritance = false;
    for &(child, parent_a, parent_b) in &first_children {
        let child = session.topology.group_of(child);
        let parent_a = session.topology.group_of(parent_a);
        let parent_b = session.topology.group_of(parent_b);
        if session.stroke_normal_mark[parent_a as usize] != epoch
            || session.stroke_normal_mark[parent_b as usize] != epoch
        {
            continue;
        }
        let read =
            |group: u32| DVec3::from_array(session.stroke_normal[group as usize].map(f64::from));
        let expected = (read(parent_a) + read(parent_b)).normalize_or_zero();
        assert_eq!(session.stroke_normal_mark[child as usize], epoch);
        let actual = read(child);
        assert!((actual - expected).length() < 1e-6);
        checked_inheritance = true;
        break;
    }
    assert!(
        checked_inheritance,
        "a split child must inherit its parents' stroke normals"
    );

    let record = session.end_stroke();
    assert!(!record.journal.is_empty());
    assert!(session
        .restore_topo(&record.indices, &record.before, false, &record.journal)
        .is_some());
    assert!(session
        .stroke_normal_mark
        .iter()
        .all(|&mark| mark == u32::MAX));
    assert!(session.spine.is_empty());
    assert!(session
        .restore_topo(&record.indices, &record.after, true, &record.journal)
        .is_some());
    assert!(session
        .stroke_normal_mark
        .iter()
        .all(|&mark| mark == u32::MAX));
    assert!(session.spine.is_empty());
}

#[test]
fn add_strength_scales_the_dab_dose_linearly() {
    let center = grid_vertex(8, 0, 0);
    let mut half = grid_session(8, 0.5, 0.0);
    let mut full = grid_session(8, 0.5, 0.0);
    let dab = centered_dab(2.0, BrushMode::Deposit, 0.5);
    let _ = half.dab(&dab);
    let _ = full.dab(&Dab {
        strength: 1.0,
        ..dab
    });
    let half_lift = half.group_v(half.topology.group_of(center)).z;
    let full_lift = full.group_v(full.topology.group_of(center)).z;
    let ratio = full_lift / half_lift;
    assert!(
        (1.8..=2.2).contains(&ratio),
        "strength 0.5 lifted {half_lift} and strength 1.0 lifted {full_lift} ({ratio}x)"
    );
}

#[test]
fn gentle_relax_moves_a_cusp_far_less_than_smooth() {
    let center = grid_vertex(6, 0, 0);
    let mut smooth = grid_session(6, 0.5, 2.0);
    let mut relax = grid_session(6, 0.5, 2.0);
    let smooth_start = smooth.group_v(smooth.topology.group_of(center));
    let relax_start = relax.group_v(relax.topology.group_of(center));
    let smooth_dab = centered_dab(2.5, BrushMode::Smooth, 1.0);
    let relax_dab = centered_dab(2.5, BrushMode::Relax, 1.0);
    for _ in 0..8 {
        let _ = smooth.dab(&smooth_dab);
        let _ = relax.dab(&relax_dab);
    }
    let smooth_move = (smooth.group_v(smooth.topology.group_of(center)) - smooth_start).length();
    let relax_move = (relax.group_v(relax.topology.group_of(center)) - relax_start).length();
    assert!(smooth_move > 0.05, "Smooth moved the cusp {smooth_move} mm");
    // Relax removes fine grain but keeps much more of the cusp than Smooth.
    assert!(
        relax_move < smooth_move * 0.5,
        "Relax moved the cusp {relax_move} mm; Smooth moved it {smooth_move} mm"
    );
}

#[test]
fn single_relax_step_applies_one_bilaplacian_pair() {
    let mut session = grid_session(1, 1.0, 0.1);
    let center = grid_vertex(1, 0, 0);
    let before = session.v(center);
    let dab = Dab {
        center: before,
        ..centered_dab(3.0, BrushMode::Relax, 1.0)
    };
    let _ = session.dab(&dab);
    // The only interior vertex moves; its boundary neighbours stay at z=0.
    // A +1/2 then -1/2 pair leaves 3/4 of the initial height. One full
    // brush interval applies 0.55 of that correction.
    let expected_z = before.z * (1.0 - 0.55 * 0.25);
    let after = session.v(center);
    assert!((after.z - expected_z).abs() < 1e-8);
    assert_eq!(after.x, before.x);
    assert_eq!(after.y, before.y);
    for vertex in 0..session.vertex_count() as u32 {
        if vertex != center {
            assert_eq!(session.v(vertex).z, 0.0);
        }
    }
}

#[test]
fn long_low_strength_relax_reduces_a_cusp_with_a_bounded_swept_stamp() {
    let center = grid_vertex(8, 0, 0);
    let radius = 2.5;
    // A long traveled step carries one capped time interval; its average stamp
    // stays at or below one rather than multiplying dose by travel length.
    let strength = 0.35;
    let mut session = grid_session(8, 0.5, 1.5);
    session.start_stroke();
    let first = session.dab_at_ray(
        DVec3::new(-1.5, 0.0, 4.0),
        -DVec3::Z,
        radius,
        strength,
        BrushMode::Relax,
        false,
    );
    assert!(first.complete && first.hit.is_some());
    let before = session.group_v(session.topology.group_of(center)).z;

    let swept = session.dab_at_ray(
        DVec3::new(1.5, 0.0, 4.0),
        -DVec3::Z,
        radius,
        strength,
        BrushMode::Relax,
        false,
    );
    assert!(swept.complete && swept.hit.is_some());
    let peak_stamp_weight = session
        .weights
        .iter()
        .map(|&(_, weight)| weight)
        .fold(0.0, f64::max);
    assert!(
        peak_stamp_weight > 0.0 && peak_stamp_weight <= 1.0 + 1e-12,
        "fixture must exercise a bounded swept mean stamp, got {peak_stamp_weight}"
    );
    let after = session.group_v(session.topology.group_of(center)).z;
    assert!(
        after < before,
        "swept Relax did not reduce the central cusp"
    );
}

#[test]
fn long_ray_jump_sweeps_only_its_reachable_tail_and_records_exact_history() {
    let half_cells = 12;
    // The raised target is 0.71 mm from the new tail at x=1, but 1.58 mm
    // from the old two-radius tail at x=2. Keep the full 3D cusp height in
    // that footprint distance.
    let target = grid_vertex(half_cells, 1, 0);
    let untouched = grid_vertex(half_cells, 0, 0);
    let radius = 1.0;
    let strength = 1.0;
    let mut endpoint_only = grid_session_with_offset_spike(half_cells, 0.5, 0.5, 1);
    let target_group = endpoint_only.topology.group_of(target);
    let initial_target = endpoint_only.group_v(target_group);
    endpoint_only.start_stroke();
    endpoint_only.remesh_armed = false;
    let endpoint = endpoint_only.dab_at_ray(
        DVec3::new(4.0, 0.0, 4.0),
        -DVec3::Z,
        radius,
        strength,
        BrushMode::Deposit,
        false,
    );
    assert!(endpoint.complete && endpoint.hit.is_some());
    assert_eq!(endpoint_only.group_v(target_group), initial_target);

    let mut session = grid_session_with_offset_spike(half_cells, 0.5, 0.5, 1);
    let original_verts = session.verts.clone();
    let original_tris = session.tris.clone();
    let target_group = session.topology.group_of(target);
    let untouched_group = session.topology.group_of(untouched);
    session.start_stroke();
    // Isolate the path's dose from the independent remesher. The record still
    // exercises the same public stroke path and exact position undo contract.
    session.remesh_armed = false;
    let first = session.dab_at_ray(
        DVec3::new(-4.0, 0.0, 4.0),
        -DVec3::Z,
        radius,
        strength,
        BrushMode::Deposit,
        false,
    );
    assert!(first.complete && first.hit.is_some());
    let jump = session.dab_at_ray(
        DVec3::new(4.0, 0.0, 4.0),
        -DVec3::Z,
        radius,
        strength,
        BrushMode::Deposit,
        false,
    );
    assert!(jump.complete && jump.hit.is_some());

    let target_after = session.group_v(target_group);
    assert!(
        target_after.z > initial_target.z + 1e-3,
        "reachable tail did not touch the spike: {target_after:?}"
    );
    assert_eq!(
        session.group_v(untouched_group).z,
        0.0,
        "the long gap before the reachable tail must not be caught up"
    );
    let record = session.end_stroke();
    assert!(record.indices.contains(&target));
    let after_verts = session.verts.clone();
    let after_tris = session.tris.clone();
    session
        .restore_topo(&record.indices, &record.before, false, &record.journal)
        .expect("undo accepts the current stroke revision");
    assert_eq!(session.verts, original_verts);
    assert_eq!(session.tris, original_tris);
    session
        .restore_topo(&record.indices, &record.after, true, &record.journal)
        .expect("redo accepts the restored base revision");
    assert_eq!(session.verts, after_verts);
    assert_eq!(session.tris, after_tris);
}

#[test]
fn preserve_skirt_knife_keeps_incremental_normals_current_through_history() {
    let mut session = irregular_sheet_session();
    let original_verts = session.verts.clone();
    let original_tris = session.tris.clone();
    session.set_brush_tip(TipStamp::Knife);
    session.start_stroke();

    for (x, hold) in [(-0.8, false), (-0.8, true), (0.9, false)] {
        session.set_dab_elapsed_ms(DWELL_FULL_DOSE_MS);
        session.set_preserve_skirt(true);
        let result = session.dab_at_ray(
            DVec3::new(x, 0.0, 10.0),
            -DVec3::Z,
            2.4,
            0.35,
            BrushMode::Deposit,
            hold,
        );
        assert!(result.complete && result.hit.is_some());
    }

    let record = session.end_stroke();
    assert!(!record.indices.is_empty());
    assert!(record.journal.encoded_size_words() > 0);
    let after_verts = session.verts.clone();
    let after_tris = session.tris.clone();
    let incremental_brush = session.brush_normals().to_vec();
    let incremental_display = session.normals().to_vec();

    session.refresh_all_normals();
    assert_live_normal_buffers_match(
        &session,
        &incremental_brush,
        &incremental_display,
        "stroke-final cache versus full refresh",
    );
    let expected_brush = session.brush_normals().to_vec();
    let expected_display = session.normals().to_vec();

    session
        .restore_topo(&record.indices, &record.before, false, &record.journal)
        .expect("undo accepts the current stroke revision");
    assert_eq!(session.verts, original_verts);
    assert_eq!(session.tris, original_tris);
    session
        .restore_topo(&record.indices, &record.after, true, &record.journal)
        .expect("redo accepts the restored base revision");
    assert_eq!(session.verts, after_verts);
    assert_eq!(session.tris, after_tris);
    assert_live_normal_buffers_match(
        &session,
        &expected_brush,
        &expected_display,
        "redo cache versus full refresh",
    );
}

#[test]
fn appended_group_without_overlay_rows_has_no_base_neighbors_or_faces() {
    let verts = [0.0_f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
    let mut topology = SurfaceTopology::new(&verts, &[0, 1, 2]).expect("valid mesh fixture");
    let appended = topology.append_group(3);
    topology.set_neighbors(0, Vec::new());
    topology.set_incident(0, Vec::new());

    assert!(topology.neighbors(appended).is_empty());
    assert!(topology.incident_triangles(appended).is_empty());
}

#[test]
fn raycast_interpolates_the_display_normals_across_a_face() {
    let verts = vec![
        0.0_f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, -1.0, 1.0,
    ];
    let mut session =
        SculptSession::new(verts, vec![0, 1, 2, 1, 0, 3]).expect("valid mesh fixture");
    let (hit, normal) = session
        .raycast(DVec3::new(0.2, 0.2, 1.0), -DVec3::Z)
        .expect("the ray hits the front face");
    let expected =
        (session.display_n(0) * 0.6 + session.display_n(1) * 0.2 + session.display_n(2) * 0.2)
            .normalize_or_zero();
    assert!((hit - DVec3::new(0.2, 0.2, 0.0)).length() < 1e-12);
    assert!((normal - expected).length() < 1e-12);
    assert!(normal.y > 0.1, "the shading normal was {normal:?}");
}

#[test]
fn stroke_ray_uses_welded_normals_while_public_ray_keeps_the_crease() {
    let mut session = cube_crease_session();
    let origin = DVec3::new(0.8, 0.8, 3.0);
    let direction = -DVec3::Z;
    let (display_hit, display_normal) = session
        .raycast(origin, direction)
        .expect("the public ray hits the top face");
    let triangle = session.hit_triangle.expect("public hit records its face");
    let corners = session.topology.triangle(triangle).expect("face is live");
    let (a, b, c) = (
        session.group_v(corners[0]),
        session.group_v(corners[1]),
        session.group_v(corners[2]),
    );
    let face = (b - a).cross(c - a);
    let area = face.length_squared();
    let wa = (c - b).cross(display_hit - b).dot(face) / area;
    let wb = (a - c).cross(display_hit - c).dot(face) / area;
    let wc = 1.0 - wa - wb;
    let expected_display = (session.display_n(corners[0]) * wa
        + session.display_n(corners[1]) * wb
        + session.display_n(corners[2]) * wc)
        .normalize_or_zero();
    let expected_brush = (session.group_n(session.topology.group_of(corners[0])) * wa
        + session.group_n(session.topology.group_of(corners[1])) * wb
        + session.group_n(session.topology.group_of(corners[2])) * wc)
        .normalize_or_zero();
    let (brush_hit, brush_normal) = session
        .raycast_visible_for_brush(origin, direction, SculptRayConstraints::default())
        .expect("the stroke ray hits the top face");

    assert!((display_hit - brush_hit).length() < 1e-12);
    assert!((display_normal - expected_display).length() < 1e-12);
    assert!((brush_normal - expected_brush).length() < 1e-12);
    assert!(display_normal.dot(DVec3::Z) > 0.99);
    assert!(brush_normal.dot(display_normal) < 0.95);

    session.start_stroke();
    session.remesh_armed = false;
    let stroke = session.dab_at_ray(origin, direction, 2.5, 0.2, BrushMode::Deposit, false);
    let (_, returned_normal) = stroke.hit.expect("the stroke still returns its ray hit");
    let (_, current_brush_normal) = session
        .raycast_visible_for_brush(origin, direction, SculptRayConstraints::default())
        .expect("the stroke ray remains on the edited surface");
    let (_, current_display_normal) = session
        .raycast(origin, direction)
        .expect("the public display ray remains available");
    assert!((returned_normal - current_brush_normal).length() < 1e-12);
    assert!(current_display_normal.dot(current_brush_normal) < 0.95);
}

#[test]
fn smooth_reduces_the_curvature_of_a_synthetic_spike() {
    let center = grid_vertex(6, 0, 0);
    let mut session = grid_session(6, 0.5, 2.0);
    let before = center_curvature(&session, center).abs();
    let dab = centered_dab(2.5, BrushMode::Smooth, 1.0);
    // Eighteen dabs are about three and a half passes of a 2.5 mm ball, so
    // this is three and a half fairing solves at full strength, not eighteen.
    for _ in 0..18 {
        assert!(!session.dab(&dab).is_empty());
    }
    let after = center_curvature(&session, center).abs();
    assert!(
        after < before * 0.90,
        "curvature changed from {before} to {after}"
    );
}

/// A square post with vertical walls on a flat sheet, the shape of a scan
/// marker. The sheet has no vertices under the post, so its walls are real
/// vertical faces rather than a steep step in a heightfield. Returns the
/// session with the vertex id of the post's top rim and of its top centre.
fn post_session(half: isize, post: isize, spacing: f64, height: f64) -> (SculptSession, u32, u32) {
    use std::collections::HashMap;
    fn vertex(
        ids: &mut HashMap<(isize, isize, bool), u32>,
        verts: &mut Vec<f32>,
        key: (isize, isize, bool),
        spacing: f64,
        height: f64,
    ) -> u32 {
        *ids.entry(key).or_insert_with(|| {
            let (i, j, top) = key;
            let z = if top { height } else { 0.0 };
            let id = (verts.len() / 3) as u32;
            verts.extend_from_slice(&[
                (i as f64 * spacing) as f32,
                (j as f64 * spacing) as f32,
                z as f32,
            ]);
            id
        })
    }
    let inside = |i: isize, j: isize| (-post..post).contains(&i) && (-post..post).contains(&j);
    let mut ids = HashMap::new();
    let mut verts = Vec::new();
    let mut tris = Vec::new();
    for j in -half..half {
        for i in -half..half {
            let top = inside(i, j);
            let a = vertex(&mut ids, &mut verts, (i, j, top), spacing, height);
            let b = vertex(&mut ids, &mut verts, (i + 1, j, top), spacing, height);
            let c = vertex(&mut ids, &mut verts, (i, j + 1, top), spacing, height);
            let d = vertex(&mut ids, &mut verts, (i + 1, j + 1, top), spacing, height);
            tris.extend_from_slice(&[a, b, d, a, d, c]);
        }
    }
    // Each footprint edge joins the sheet below to the cap above. The winding
    // is chosen so every wall faces out of the post.
    let mut walls = Vec::new();
    for k in -post..post {
        walls.push(((k, -post), (k + 1, -post), (0.0, -1.0)));
        walls.push(((k + 1, post), (k, post), (0.0, 1.0)));
        walls.push(((-post, k + 1), (-post, k), (-1.0, 0.0)));
        walls.push(((post, k), (post, k + 1), (1.0, 0.0)));
    }
    for (p, q, outward) in walls {
        let bp = vertex(&mut ids, &mut verts, (p.0, p.1, false), spacing, height);
        let bq = vertex(&mut ids, &mut verts, (q.0, q.1, false), spacing, height);
        let tp = vertex(&mut ids, &mut verts, (p.0, p.1, true), spacing, height);
        let tq = vertex(&mut ids, &mut verts, (q.0, q.1, true), spacing, height);
        // The normal of this quad is the edge direction turned by 90 degrees.
        let (dx, dy) = ((q.0 - p.0) as f64, (q.1 - p.1) as f64);
        let facing = dy * outward.0 - dx * outward.1;
        if facing >= 0.0 {
            tris.extend_from_slice(&[bp, bq, tq, bp, tq, tp]);
        } else {
            tris.extend_from_slice(&[bp, tq, bq, bp, tp, tq]);
        }
    }
    let rim = ids[&(post, 0, true)];
    let centre = ids[&(0, 0, true)];
    let session = SculptSession::new(verts, tris).expect("valid post fixture");
    (session, rim, centre)
}

#[test]
fn smooth_brings_a_post_wall_down_instead_of_leaving_a_well() {
    let (mut session, rim, centre) = post_session(12, 3, 0.25, 2.0);
    let height =
        |session: &SculptSession, vertex: u32| session.group_v(session.topology.group_of(vertex)).z;
    let (rim_before, centre_before) = (height(&session, rim), height(&session, centre));
    // The operator presses on the post's top, so the brush sits there.
    let dab = Dab {
        center: DVec3::new(0.0, 0.0, 2.0),
        ..centered_dab(2.0, BrushMode::Smooth, 1.0)
    };
    for _ in 0..18 {
        assert!(!session.dab(&dab).is_empty());
    }
    let (rim_after, centre_after) = (height(&session, rim), height(&session, centre));
    assert!(
        rim_after < rim_before - 0.5,
        "the post's top rim stayed at {rim_after} (was {rim_before}); the walls did not come down"
    );
    // A well is the centre sinking under a rim that stays up.
    assert!(
        centre_after > rim_after - 0.5,
        "a well: the centre fell to {centre_after} under a rim at {rim_after}"
    );
    assert!(centre_before > centre_after);
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

#[test]
fn add_dose_tracks_elapsed_brush_time_and_caps_a_stall() {
    let center = grid_vertex(8, 0, 0);
    let dab = centered_dab(1.0, BrushMode::Deposit, 0.25);
    let mut half = grid_session(8, 0.5, 0.0);
    let mut full = grid_session(8, 0.5, 0.0);
    let mut delayed = grid_session(8, 0.5, 0.0);
    for session in [&mut half, &mut full, &mut delayed] {
        session.remesh_armed = false;
    }
    half.set_dab_elapsed_ms(DWELL_FULL_DOSE_MS * 0.5);
    full.set_dab_elapsed_ms(DWELL_FULL_DOSE_MS);
    delayed.set_dab_elapsed_ms(DWELL_FULL_DOSE_MS * 4.0);
    let _ = half.dab(&dab);
    let _ = full.dab(&dab);
    let _ = delayed.dab(&dab);
    let half_lift = half.group_v(half.topology.group_of(center)).z;
    let full_lift = full.group_v(full.topology.group_of(center)).z;
    let delayed_lift = delayed.group_v(delayed.topology.group_of(center)).z;
    assert!(half_lift > 0.0 && full_lift > half_lift);
    assert!((full_lift / half_lift - 2.0).abs() < 0.08);
    assert_eq!(delayed_lift, full_lift, "a stall is capped at one dose");
}

#[test]
fn travel_shares_one_time_dose_over_its_average_stamp() {
    let center = grid_vertex(8, 0, 0);
    let radius = 1.0;
    let strength = 0.25;
    let mut stationary = grid_session(8, 0.5, 0.0);
    stationary.remesh_armed = false;
    stationary.set_dab_elapsed_ms(DWELL_FULL_DOSE_MS);
    let _ = stationary.dab(&centered_dab(radius, BrushMode::Deposit, strength));
    let stationary_lift = stationary.group_v(stationary.topology.group_of(center)).z;

    let mut traveling = grid_session(8, 0.5, 0.0);
    traveling.start_stroke();
    traveling.remesh_armed = false;
    traveling.set_dab_elapsed_ms(0.0);
    let first = traveling.dab_at_ray(
        DVec3::new(-1.5, 0.0, 4.0),
        -DVec3::Z,
        radius,
        strength,
        BrushMode::Deposit,
        false,
    );
    assert!(first.complete && first.hit.is_some());
    traveling.set_dab_elapsed_ms(DWELL_FULL_DOSE_MS);
    let swept = traveling.dab_at_ray(
        DVec3::new(1.5, 0.0, 4.0),
        -DVec3::Z,
        radius,
        strength,
        BrushMode::Deposit,
        false,
    );
    assert!(swept.complete && swept.hit.is_some());
    let traveled_lift = traveling.group_v(traveling.topology.group_of(center)).z;
    let peak_stamp = traveling
        .weights
        .iter()
        .map(|&(_, weight)| weight)
        .fold(0.0, f64::max);
    assert!(traveled_lift > 0.0);
    assert!(traveled_lift < stationary_lift);
    assert!(peak_stamp > 0.0 && peak_stamp <= 1.0 + 1e-12);
}

#[test]
fn zero_or_invalid_brush_time_does_not_displace_the_surface() {
    for mode in [
        BrushMode::Deposit,
        BrushMode::Erode,
        BrushMode::Smooth,
        BrushMode::Relax,
        BrushMode::Flatten,
    ] {
        let dab = centered_dab(1.0, mode, 0.8);
        for elapsed in [0.0, f64::NAN, f64::INFINITY] {
            let mut session = grid_session(8, 0.5, 1.0);
            session.remesh_armed = false;
            let before = session.verts.clone();
            session.set_dab_elapsed_ms(elapsed);
            let moved = session.dab(&dab);
            assert!(moved.is_empty(), "{mode:?} moved with elapsed={elapsed}");
            assert_eq!(session.verts, before, "{mode:?} changed coordinates");
        }
    }
}

#[test]
fn add_remove_amplitude_is_radius_relative_and_time_scaled() {
    let mut session = grid_session(5, 0.5, 0.0);
    let radius = 2.5;
    let strength = 0.8;
    session.start_stroke();
    let _ = session.dab(&Dab {
        center: DVec3::ZERO,
        radius,
        strength,
        view: DVec3::new(0.0, 0.0, -1.0),
        mode: BrushMode::Deposit,
    });
    let expected = layer_depth(radius, strength);
    assert!((f64::from(session.live_kin.amplitude) - expected).abs() < 1e-7);
}

#[test]
fn smooth_and_relax_have_a_shared_full_time_rate() {
    let center = grid_vertex(6, 0, 0);
    for mode in [BrushMode::Smooth, BrushMode::Relax] {
        let mut partial_session = grid_session(6, 0.5, 1.5);
        let mut full_session = grid_session(6, 0.5, 1.5);
        partial_session.remesh_armed = false;
        full_session.remesh_armed = false;
        let dab = centered_dab(2.5, mode, 0.4);
        partial_session.set_dab_elapsed_ms(DWELL_FULL_DOSE_MS * 0.5);
        full_session.set_dab_elapsed_ms(DWELL_FULL_DOSE_MS);
        let _ = partial_session.dab(&dab);
        let _ = full_session.dab(&dab);
        let partial = partial_session
            .group_v(partial_session.topology.group_of(center))
            .z;
        let full = full_session
            .group_v(full_session.topology.group_of(center))
            .z;
        assert!(
            full < partial && partial < 1.5,
            "{mode:?} must scale with time"
        );
        assert!((full_session.live_kin.gain - 0.22).abs() < 1e-7);
        assert!((partial_session.live_kin.gain - 0.11).abs() < 1e-7);
    }
}

#[test]
fn shape_preserving_fairing_retains_a_broad_round_form() {
    struct RingSurface {
        positions: Vec<DVec3>,
        neighbors: Vec<Vec<u32>>,
    }
    impl crate::FairingSurface for RingSurface {
        fn vertex_count(&self) -> usize {
            self.positions.len()
        }
        fn position(&self, vertex: u32) -> DVec3 {
            self.positions[vertex as usize]
        }
        fn vertex_area(&self, _vertex: u32) -> f64 {
            1.0
        }
        fn neighbors(&self, vertex: u32) -> &[u32] {
            &self.neighbors[vertex as usize]
        }
    }

    let count = 64usize;
    let mut positions = Vec::with_capacity(count);
    let mut neighbors = Vec::with_capacity(count);
    for index in 0..count {
        let angle = std::f64::consts::TAU * index as f64 / count as f64;
        positions.push(DVec3::new(
            10.0 * angle.cos(),
            10.0 * angle.sin(),
            2.0 * angle.cos(),
        ));
        neighbors.push(vec![
            ((index + count - 1) % count) as u32,
            ((index + 1) % count) as u32,
        ]);
    }
    let surface = RingSurface {
        positions,
        neighbors,
    };
    let selection: Vec<(u32, f64)> = (0..count as u32).map(|vertex| (vertex, 1.0)).collect();
    let mut ordinary = Vec::new();
    let mut preserving = Vec::new();
    crate::fair_selection(
        &surface,
        &selection,
        4.0,
        &mut crate::FairingScratch::default(),
        &mut ordinary,
    );
    crate::fair_selection_preserving(
        &surface,
        &selection,
        4.0,
        &mut crate::FairingScratch::default(),
        &mut preserving,
    );
    let ordinary_peak = ordinary
        .iter()
        .find(|&&(vertex, _)| vertex == 0)
        .expect("the fairing output includes the first ring vertex")
        .1
        .z;
    let preserved_peak = preserving
        .iter()
        .find(|&&(vertex, _)| vertex == 0)
        .expect("the preserving output includes the first ring vertex")
        .1
        .z;
    assert!(preserved_peak > ordinary_peak + 0.1);
    assert!(preserved_peak <= 2.0 + 1e-8);
}

#[test]
fn brush_preserving_fairing_starts_cold_after_a_general_solve_on_other_geometry() {
    use crate::FairingSurface;

    struct GridSurface {
        positions: Vec<DVec3>,
        neighbors: Vec<Vec<u32>>,
    }
    impl FairingSurface for GridSurface {
        fn vertex_count(&self) -> usize {
            self.positions.len()
        }
        fn position(&self, vertex: u32) -> DVec3 {
            self.positions[vertex as usize]
        }
        fn vertex_area(&self, _vertex: u32) -> f64 {
            1.0
        }
        fn neighbors(&self, vertex: u32) -> &[u32] {
            &self.neighbors[vertex as usize]
        }
    }

    fn surface(side: usize, previous: bool) -> GridSurface {
        let mut positions = Vec::with_capacity(side * side);
        let mut neighbors = Vec::with_capacity(side * side);
        for y in 0..side {
            for x in 0..side {
                let x_f = x as f64;
                let y_f = y as f64;
                let z = if previous {
                    f64::from(((x * 17 + y * 31) % 23) as u32) / 3.0
                } else {
                    2.0 * (x_f * 0.17).sin() * (y_f * 0.13).cos()
                        + 0.25 * (x_f * 0.73 + y_f * 0.41).sin()
                };
                positions.push(DVec3::new(x_f, y_f, z));

                let id = (y * side + x) as u32;
                let mut row = Vec::with_capacity(4);
                if y > 0 {
                    row.push(id - side as u32);
                }
                if x > 0 {
                    row.push(id - 1);
                }
                if x + 1 < side {
                    row.push(id + 1);
                }
                if y + 1 < side {
                    row.push(id + side as u32);
                }
                neighbors.push(row);
            }
        }
        GridSurface {
            positions,
            neighbors,
        }
    }

    let previous = surface(23, true);
    let current = surface(23, false);
    let selection: Vec<(u32, f64)> = (0..current.vertex_count() as u32)
        .map(|vertex| (vertex, 1.0))
        .collect();
    let mut reused_scratch = crate::FairingScratch::default();
    let mut previous_targets = Vec::new();
    crate::fair_selection(
        &previous,
        &selection,
        10.0,
        &mut reused_scratch,
        &mut previous_targets,
    );
    assert!(!previous_targets.is_empty());

    let mut reused_targets = Vec::new();
    crate::fair_selection_preserving(
        &current,
        &selection,
        10.0,
        &mut reused_scratch,
        &mut reused_targets,
    );
    let mut fresh_targets = Vec::new();
    crate::fair_selection_preserving(
        &current,
        &selection,
        10.0,
        &mut crate::FairingScratch::default(),
        &mut fresh_targets,
    );

    assert_eq!(reused_targets, fresh_targets);
}

#[test]
fn breaking_ray_continuity_keeps_the_current_stroke_open() {
    let mut session = grid_session(8, 0.5, 0.0);
    session.start_stroke();
    let result = session.dab_at_ray(
        DVec3::new(0.0, 0.0, 2.0),
        -DVec3::Z,
        1.5,
        0.2,
        BrushMode::Deposit,
        false,
    );
    assert!(result.complete);
    assert!(result.hit.is_some());
    assert!(session.stroke_path.is_some());
    assert!(!session.stroke_indices.is_empty());

    session.break_stroke_path();

    assert!(session.stroke_path.is_none());
    assert!(session.dab_path.is_empty());
    assert!(!session.stroke_indices.is_empty());
    assert!(session.remesh_armed);
    assert!(!session.end_stroke().indices.is_empty());
}

#[test]
fn remove_wall_guard_keeps_tangent_motion_for_a_sub_epsilon_support_gap() {
    let mut session = grid_session(4, 1.0, 0.0);
    let group = session.topology.group_of(grid_vertex(4, 0, 0));
    let reference = session.reference_group_v(group);
    let anchor = [reference.x as f32, reference.y as f32, reference.z as f32];
    session.reference_wall_mm[group as usize] = 1.2;
    session.reference_wall_at[group as usize] = anchor;
    session.wall_facing = Some(1.0);

    let axis = session.sheet_push(group, 1.0).normalize_or_zero();
    assert_eq!(axis, DVec3::Z);
    let wall = session.reference_group_wall_mm(group);
    let minimum_support = -((wall - 0.8) * 0.5).max(0.0);
    let ulp_scale = f64::EPSILON * minimum_support.abs();
    let current_support = minimum_support + 8.0 * ulp_scale;
    let candidate_support = minimum_support - 2.0 * ulp_scale;
    let tangent = DVec3::X;
    let current = reference + axis * current_support;
    let candidate = reference + axis * candidate_support + tangent * 0.02;

    let measured_current = (current - reference).dot(axis);
    let measured_candidate = (candidate - reference).dot(axis);
    let denominator = measured_current - measured_candidate;
    assert!(measured_candidate < minimum_support);
    assert!(denominator > 0.0 && denominator < 1e-15);

    let guarded = session.guard_remove_wall(group, current, candidate);
    let guarded_support = (guarded - reference).dot(axis);
    let guarded_tangent = (guarded - current).dot(tangent);
    let expected_fraction = (measured_current - minimum_support) / denominator;
    assert!((guarded_support - minimum_support).abs() < 1e-14);
    assert!((guarded_tangent - 0.02 * expected_fraction).abs() < 1e-12);
    assert!(guarded_tangent > 0.01, "guard discarded tangent motion");
}

#[test]
fn public_erode_respects_an_opposing_wall_once_the_caller_prepared_the_probe() {
    let half_cells = 4;
    let mut session = closed_slab_session(half_cells, 1.0, 1.2);
    let top_raw = (2 * half_cells + 1).pow(2) as u32 + grid_vertex(half_cells, 0, 0);
    let group = session.topology.group_of(top_raw);
    // The probe belongs to the caller's preparation step. Building it inside
    // the first carve measured a whole-mesh distance field while the operator
    // was already painting, so the guard only ever reads a prepared probe.
    session.prepare_wall_probe();
    session.start_stroke();
    session.remesh_armed = false;
    let dab = centered_dab(2.0, BrushMode::Erode, 1.0);

    let _ = session.dab(&dab);
    assert!(
        session.wall_probe.is_some(),
        "the caller prepared the probe"
    );
    let wall = session.reference_group_wall_mm(group);
    assert!((1.1..=1.3).contains(&wall), "measured wall was {wall} mm");
    assert!(!session.reference_wall_mm[group as usize].is_nan());

    for _ in 0..24 {
        let _ = session.dab(&dab);
    }
    let reference = session.reference_group_v(group);
    let support = (session.group_v(group) - reference).dot(DVec3::Z);
    let reserve = (wall - 0.8) * 0.5;
    assert!(support < -0.1, "the brush removed material by {support} mm");
    assert!(
        support >= -reserve - 0.03,
        "Erode crossed its opposing-wall reserve: {support} mm vs {reserve} mm"
    );
    assert!(session.is_apply_safe());
}

#[test]
fn open_sheet_erode_keeps_the_probe_cap_and_remains_editable() {
    let half_cells = 4;
    let mut session = grid_session(half_cells, 0.5, 0.0);
    let raw = grid_vertex(half_cells, 0, 0);
    let group = session.topology.group_of(raw);
    session.start_stroke();
    session.remesh_armed = false;
    let _ = session.dab(&centered_dab(1.5, BrushMode::Erode, 1.0));
    assert_eq!(session.reference_group_wall_mm(group), 10.0);
    assert!(
        session.group_v(group).z < -1e-3,
        "open surface remains removable"
    );
}

#[test]
fn erode_after_smooth_remesh_undo_and_redo_retains_wall_protection() {
    let half_cells = 3;
    let mut session = closed_slab_session(half_cells, 1.0, 1.2);
    let base_groups = session.topology.group_count();
    let raw = (2 * half_cells + 1).pow(2) as u32 + grid_vertex(half_cells, 0, 0);
    let group = session.topology.group_of(raw);
    session.prepare_wall_probe();
    assert!(session.prime_wall_region(DVec3::ZERO, 2.0, 64) > 0);

    session.start_stroke();
    let _ = session.dab(&Dab {
        center: DVec3::ZERO,
        radius: 1.8,
        strength: 0.4,
        view: DVec3::new(0.0, 0.0, -1.0),
        mode: BrushMode::Smooth,
    });
    let smooth = session.end_stroke();
    assert!(
        !smooth.journal.added_verts.is_empty(),
        "Smooth exercised remesh"
    );
    assert!(session.reference_wall_mm[group as usize].is_finite());

    assert!(session
        .restore_topo(&smooth.indices, &smooth.before, false, &smooth.journal)
        .is_some());
    assert_eq!(session.reference_wall_mm.len(), base_groups);
    assert!(session
        .restore_topo(&smooth.indices, &smooth.after, true, &smooth.journal)
        .is_some());
    assert_eq!(
        session.reference_wall_mm.len(),
        base_groups + smooth.journal.added_verts.len()
    );
    assert!(session
        .restore_topo(&smooth.indices, &smooth.before, false, &smooth.journal)
        .is_some());

    session.start_stroke();
    let _ = session.dab(&centered_dab(2.0, BrushMode::Erode, 1.0));
    let wall = session.reference_group_wall_mm(group);
    let support = (session.group_v(group) - session.reference_group_v(group)).dot(DVec3::Z);
    let reserve = (wall - 0.8) * 0.5;
    assert!(wall < 1.3, "wall reading survived topology history: {wall}");
    assert!(
        support >= -reserve - 0.03,
        "first post-undo Remove crossed its wall"
    );
}
