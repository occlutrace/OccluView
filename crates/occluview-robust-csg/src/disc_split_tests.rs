#![allow(clippy::cast_possible_truncation, clippy::expect_used)]

use super::*;
use manifold_csg::Manifold;
use std::ops::Range;

#[allow(clippy::cast_possible_truncation)]
fn append_box(mesh: &mut RobustMesh, min: [f64; 3], max: [f64; 3]) -> Range<usize> {
    let base = mesh.positions.len() as u64;
    let index_start = mesh.indices.len();
    mesh.positions.extend([
        [min[0], min[1], min[2]],
        [max[0], min[1], min[2]],
        [max[0], max[1], min[2]],
        [min[0], max[1], min[2]],
        [min[0], min[1], max[2]],
        [max[0], min[1], max[2]],
        [max[0], max[1], max[2]],
        [min[0], max[1], max[2]],
    ]);
    mesh.indices.extend([
        base,
        base + 2,
        base + 1,
        base,
        base + 3,
        base + 2,
        base + 4,
        base + 5,
        base + 6,
        base + 4,
        base + 6,
        base + 7,
        base,
        base + 1,
        base + 5,
        base,
        base + 5,
        base + 4,
        base + 3,
        base + 7,
        base + 6,
        base + 3,
        base + 6,
        base + 2,
        base,
        base + 4,
        base + 7,
        base,
        base + 7,
        base + 3,
        base + 1,
        base + 2,
        base + 6,
        base + 1,
        base + 6,
        base + 5,
    ]);
    index_start..mesh.indices.len()
}

fn empty_mesh() -> RobustMesh {
    RobustMesh {
        positions: Vec::new(),
        indices: Vec::new(),
    }
}

fn reverse_faces(mesh: &mut RobustMesh, indices: Range<usize>) {
    for triangle in mesh.indices[indices].chunks_exact_mut(3) {
        triangle.swap(1, 2);
    }
}

fn split_gap(split: &RobustSplit, center: [f64; 3], normal: [f64; 3]) -> f64 {
    let center = DVec3::from_array(center);
    let normal = DVec3::from_array(normal).normalize();
    let positive_min = split
        .part_a
        .positions
        .iter()
        .map(|&position| (DVec3::from_array(position) - center).dot(normal))
        .fold(f64::INFINITY, f64::min);
    let negative_max = split
        .part_b
        .positions
        .iter()
        .map(|&position| (DVec3::from_array(position) - center).dot(normal))
        .fold(f64::NEG_INFINITY, f64::max);
    positive_min - negative_max
}

fn bridge_disc() -> SeparatorDisc {
    SeparatorDisc {
        center: [0.0, 0.0, 0.0],
        normal: [1.0, 0.0, 0.0],
        kerf_mm: 0.2,
        radius_mm: 3.0,
    }
}

#[test]
fn overlapping_closed_shells_are_split_as_one_additive_solid() {
    let mut mesh = empty_mesh();
    append_box(&mut mesh, [-2.0, -1.0, -1.0], [1.0, 1.0, 1.0]);
    append_box(&mut mesh, [-1.0, -1.0, -1.0], [2.0, 1.0, 1.0]);

    let result = split_with_separator_disc(&mesh, bridge_disc());

    assert!(result.is_ok(), "{result:?}");
}

#[test]
fn three_physical_results_are_composed_into_two_logical_sides() {
    let mut mesh = empty_mesh();
    append_box(&mut mesh, [-2.0, -1.0, -1.0], [2.0, 1.0, 1.0]);
    append_box(&mut mesh, [3.0, -0.2, -0.2], [3.1, 0.2, 0.2]);

    let result = split_with_separator_disc(&mesh, bridge_disc());

    let result = result.expect("logical split");
    assert_eq!(result.report.part_a_physical_components, 2);
    assert_eq!(result.report.part_b_physical_components, 1);
    assert!(result.report.part_a_cut_loops >= 1);
    assert!(result.report.part_b_cut_loops >= 1);
}

#[test]
fn prepared_solid_is_thread_safe_and_reused_without_mutating_source() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<PreparedRobustSolid>();

    let mut mesh = empty_mesh();
    append_box(&mut mesh, [-2.0, -1.0, -1.0], [2.0, 1.0, 1.0]);
    let original = mesh.clone();
    let prepared = prepare_robust_solid(&mesh).expect("prepared");
    let first = prepared
        .split_with_separator_disc(&identity_transform(), bridge_disc())
        .expect("first split");
    let second = prepared
        .split_with_separator_disc(&identity_transform(), bridge_disc())
        .expect("second split");

    assert_eq!(first, second);
    assert_eq!(mesh, original);
}

#[test]
fn nested_inward_shell_preserves_a_real_cavity() {
    let mut mesh = empty_mesh();
    append_box(&mut mesh, [-2.0; 3], [2.0; 3]);
    let cavity = append_box(&mut mesh, [-1.0; 3], [1.0; 3]);
    reverse_faces(&mut mesh, cavity);

    let prepared = prepare_robust_solid(&mesh).expect("nested cavity");

    assert!((prepared.local_volume() - 56.0).abs() < 1.0e-8);
}

#[test]
fn globally_reversed_hollow_solid_preserves_its_cavity() {
    let mut mesh = empty_mesh();
    let outer = append_box(&mut mesh, [-2.0; 3], [2.0; 3]);
    let cavity = append_box(&mut mesh, [-1.0; 3], [1.0; 3]);
    reverse_faces(&mut mesh, cavity.clone());
    reverse_faces(&mut mesh, outer);
    reverse_faces(&mut mesh, cavity);

    let prepared = prepare_robust_solid(&mesh).expect("globally reversed hollow solid");

    assert!((prepared.local_volume() - 56.0).abs() < 1.0e-8);
}

#[test]
fn cavity_may_span_the_union_of_overlapping_additive_shells() {
    let mut mesh = empty_mesh();
    append_box(&mut mesh, [-2.0, -1.0, -1.0], [0.5, 1.0, 1.0]);
    append_box(&mut mesh, [-0.5, -1.0, -1.0], [2.0, 1.0, 1.0]);
    let cavity = append_box(&mut mesh, [-1.0, -0.5, -0.5], [1.0, 0.5, 0.5]);
    reverse_faces(&mut mesh, cavity);

    let prepared = prepare_robust_solid(&mesh).expect("cavity inside additive union");

    assert!((prepared.local_volume() - 14.0).abs() < 1.0e-8);
}

#[test]
fn globally_reversed_input_is_reoriented_as_one_model() {
    let mut mesh = empty_mesh();
    let faces = append_box(&mut mesh, [-1.0; 3], [1.0; 3]);
    reverse_faces(&mut mesh, faces);

    let prepared = prepare_robust_solid(&mesh).expect("globally reversed solid");

    assert!((prepared.local_volume() - 8.0).abs() < 1.0e-8);
}

#[test]
fn uncontained_mixed_winding_is_rejected_deterministically() {
    let mut mesh = empty_mesh();
    append_box(&mut mesh, [-1.0; 3], [1.0; 3]);
    let reversed = append_box(&mut mesh, [3.0, -1.0, -1.0], [5.0, 1.0, 1.0]);
    reverse_faces(&mut mesh, reversed);

    let first = prepare_robust_solid(&mesh).expect_err("uncontained inward shell");
    let second = prepare_robust_solid(&mesh).expect_err("deterministic refusal");

    assert_eq!(first, second);
    assert!(matches!(
        first,
        RobustCsgError::AmbiguousShellWinding { .. }
    ));
}

#[test]
fn separately_indexed_face_touching_shells_split_as_one_logical_work() {
    let mut mesh = empty_mesh();
    append_box(&mut mesh, [-2.0, -1.0, -1.0], [0.0, 1.0, 1.0]);
    append_box(&mut mesh, [0.0, -1.0, -1.0], [2.0, 1.0, 1.0]);

    let split = split_with_separator_disc(&mesh, bridge_disc()).expect("touching union");

    assert!((split_gap(&split, bridge_disc().center, bridge_disc().normal) - 0.2).abs() < 1.0e-8);
}

#[test]
fn exact_position_triangle_soup_recovers_one_closed_shell() {
    let mut indexed = empty_mesh();
    append_box(&mut indexed, [-1.0; 3], [1.0; 3]);
    let mut soup = empty_mesh();
    for &index in &indexed.indices {
        soup.indices.push(soup.positions.len() as u64);
        soup.positions.push(indexed.positions[index as usize]);
    }

    let prepared = prepare_robust_solid(&soup).expect("STL-style soup");

    assert!((prepared.local_volume() - 8.0).abs() < 1.0e-8);
}

#[test]
fn edge_touching_triangle_soup_keeps_two_unambiguous_shells() {
    let mut indexed = empty_mesh();
    append_box(&mut indexed, [-1.0, -1.0, -1.0], [0.0, 0.0, 1.0]);
    append_box(&mut indexed, [0.0, 0.0, -1.0], [1.0, 1.0, 1.0]);
    let mut soup = empty_mesh();
    for &index in &indexed.indices {
        soup.indices.push(soup.positions.len() as u64);
        soup.positions.push(indexed.positions[index as usize]);
    }

    let prepared = prepare_robust_solid(&soup).expect("edge-touching soup shells");

    assert!((prepared.local_volume() - 4.0).abs() < 1.0e-10);
}

#[test]
fn microscopic_valid_solid_is_not_misclassified_as_a_separator_miss() {
    let mut mesh = empty_mesh();
    append_box(&mut mesh, [-1.0e-5; 3], [1.0e-5; 3]);
    let disc = SeparatorDisc {
        center: [0.0; 3],
        normal: [1.0, 0.0, 0.0],
        kerf_mm: 1.0e-5,
        radius_mm: 4.0,
    };

    let split = split_with_separator_disc(&mesh, disc).expect("microscopic split");

    assert!((split_gap(&split, disc.center, disc.normal) - disc.kerf_mm).abs() < 1.0e-12);
}

#[test]
fn microscopic_untouched_component_is_preserved_on_its_logical_side() {
    let mut mesh = empty_mesh();
    append_box(&mut mesh, [-2.0, -1.0, -1.0], [2.0, 1.0, 1.0]);
    append_box(
        &mut mesh,
        [3.0, 0.0, 0.0],
        [3.0 + 2.0e-12, 2.0e-12, 2.0e-12],
    );

    let split = split_with_separator_disc(&mesh, bridge_disc()).expect("tiny component split");

    assert_eq!(split.report.part_a_physical_components, 2);
    assert_eq!(split.report.part_b_physical_components, 1);
    assert!(split
        .part_a
        .positions
        .iter()
        .any(|position| position[0] >= 3.0));
}

#[test]
fn remote_untouched_component_may_cross_the_disc_plane_outside_its_radius() {
    let mut mesh = empty_mesh();
    append_box(&mut mesh, [-2.0, -0.5, -0.5], [2.0, 0.5, 0.5]);
    append_box(&mut mesh, [-1.0, 3.0, -0.5], [1.0, 4.0, 0.5]);
    let disc = SeparatorDisc {
        radius_mm: 1.5,
        ..bridge_disc()
    };

    let split = split_with_separator_disc(&mesh, disc)
        .expect("finite disc ignores a remote plane crossing");

    assert_eq!(split.report.part_a_physical_components, 2);
    assert_eq!(split.report.part_b_physical_components, 1);
    assert!(split
        .part_a
        .positions
        .iter()
        .any(|position| position[1] >= 3.0));
    for part in [&split.part_a, &split.part_b] {
        validate_separator_clearance(
            &RobustMesh {
                positions: part.positions.clone(),
                indices: part.indices.clone(),
            },
            disc,
            1.0e-4,
        )
        .expect("finite separator clearance");
    }
}

#[test]
fn cutting_one_component_preserves_a_remote_plane_crossing() {
    let mut mesh = empty_mesh();
    append_box(&mut mesh, [-4.0, 3.0, -0.5], [4.0, 4.0, 0.5]);
    append_box(&mut mesh, [-0.5; 3], [0.5; 3]);
    let disc = SeparatorDisc {
        radius_mm: 1.5,
        ..bridge_disc()
    };

    let split = split_with_separator_disc(&mesh, disc).expect("selected component split");

    assert_eq!(split.report.part_a_physical_components, 2);
    assert_eq!(split.report.part_b_physical_components, 1);
    assert_eq!(
        split
            .part_a
            .positions
            .iter()
            .filter(|position| position[1] >= 3.0)
            .count(),
        8
    );
}

#[test]
fn reflected_transform_preserves_a_microscopic_untouched_component() {
    let mut mesh = empty_mesh();
    append_box(&mut mesh, [-2.0, -1.0, -1.0], [2.0, 1.0, 1.0]);
    append_box(
        &mut mesh,
        [3.0, 0.0, 0.0],
        [3.0 + 2.0e-12, 2.0e-12, 2.0e-12],
    );
    let prepared = prepare_robust_solid(&mesh).expect("prepared with microscopic component");
    let reflection = [-1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0];

    let split = split_prepared_with_separator_disc(&prepared, &reflection, bridge_disc())
        .expect("reflected microscopic component remains valid");

    assert_eq!(split.report.part_a_physical_components, 1);
    assert_eq!(split.report.part_b_physical_components, 2);
    assert!(split
        .part_b
        .positions
        .iter()
        .any(|position| position[0] <= -3.0));
}

#[test]
fn reflected_nonuniform_world_transform_keeps_outward_parts_and_exact_gap() {
    let mut mesh = empty_mesh();
    append_box(&mut mesh, [-1.0; 3], [1.0; 3]);
    let prepared = prepare_robust_solid(&mesh).expect("prepared box");
    let transform = [-2.0, 0.0, 0.0, 0.0, 0.6, 0.0, 0.0, 0.0, 1.4, 7.0, -3.0, 2.0];
    let disc = SeparatorDisc {
        center: [7.0, -3.0, 2.0],
        normal: [-1.0, 0.0, 0.0],
        kerf_mm: 0.05,
        radius_mm: 4.0,
    };

    let split = split_prepared_with_separator_disc(&prepared, &transform, disc)
        .expect("reflected nonuniform split");

    assert!((split_gap(&split, disc.center, disc.normal) - disc.kerf_mm).abs() < 1.0e-8);
    for part in [&split.part_a, &split.part_b] {
        normalize_closed_mesh(&RobustMesh {
            positions: part.positions.clone(),
            indices: part.indices.clone(),
        })
        .expect("reflected output remains outward and closed");
    }
}

#[test]
fn misses_and_tangent_only_contacts_are_typed_failures() {
    let mut mesh = empty_mesh();
    append_box(&mut mesh, [-1.0; 3], [1.0; 3]);
    let prepared = prepare_robust_solid(&mesh).expect("prepared box");
    for center in [[10.0, 0.0, 0.0], [1.025, 0.0, 0.0]] {
        let error = prepared
            .split_with_separator_disc(
                &identity_transform(),
                SeparatorDisc {
                    center,
                    normal: [1.0, 0.0, 0.0],
                    kerf_mm: 0.05,
                    radius_mm: 4.0,
                },
            )
            .expect_err("miss or tangent must not manufacture two parts");
        assert!(matches!(
            error,
            RobustCsgError::SeparatorMiss | RobustCsgError::UnexpectedComponents { .. }
        ));
    }
}

#[test]
fn non_normalizable_finite_disc_normal_is_rejected_as_invalid_input() {
    let mut mesh = empty_mesh();
    append_box(&mut mesh, [-1.0; 3], [1.0; 3]);
    let prepared = prepare_robust_solid(&mesh).expect("prepared box");

    let error = prepared
        .split_with_separator_disc(
            &identity_transform(),
            SeparatorDisc {
                center: [0.0; 3],
                normal: [f64::MAX, 0.0, 0.0],
                kerf_mm: 0.05,
                radius_mm: 4.0,
            },
        )
        .expect_err("overflowing normal length must be rejected");

    assert!(matches!(error, RobustCsgError::InvalidInput { .. }));
}

#[test]
fn clearance_validation_detects_volume_crossing_without_an_inside_vertex() {
    let mut mesh = empty_mesh();
    append_box(&mut mesh, [-0.02, -2.0, -2.0], [0.02, 2.0, 2.0]);
    let disc = SeparatorDisc {
        center: [0.0; 3],
        normal: [1.0, 0.0, 0.0],
        kerf_mm: 0.2,
        radius_mm: 1.0,
    };

    let error = validate_separator_clearance(&mesh, disc, 0.001)
        .expect_err("triangle interiors cross the separator volume");

    assert_eq!(error, RobustCsgError::SeparatorClearanceLost);
}

#[test]
fn clearance_validation_allows_exact_cap_contact_and_remote_plane_crossings() {
    let disc = SeparatorDisc {
        center: [0.0; 3],
        normal: [1.0, 0.0, 0.0],
        kerf_mm: 0.2,
        radius_mm: 1.0,
    };
    let mut cap_contact = empty_mesh();
    append_box(&mut cap_contact, [0.1, -0.5, -0.5], [1.0, 0.5, 0.5]);
    let mut remote_crossing = empty_mesh();
    append_box(&mut remote_crossing, [-0.5, 2.0, -0.25], [0.5, 2.5, 0.25]);

    validate_separator_clearance(&cap_contact, disc, 0.001).expect("cap contact is valid");
    validate_separator_clearance(&remote_crossing, disc, 0.001)
        .expect("finite disc must ignore remote plane crossings");
}

#[test]
fn finite_discs_produce_two_closed_components_for_axis_and_oblique_normals() {
    let mesh = RobustMesh {
        positions: vec![
            [-1.0, -1.0, -1.0],
            [1.0, -1.0, -1.0],
            [1.0, 1.0, -1.0],
            [-1.0, 1.0, -1.0],
            [-1.0, -1.0, 1.0],
            [1.0, -1.0, 1.0],
            [1.0, 1.0, 1.0],
            [-1.0, 1.0, 1.0],
        ],
        indices: vec![
            0, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7, 0, 1, 5, 0, 5, 4, 3, 7, 6, 3, 6, 2, 0, 4, 7, 0, 7,
            3, 1, 2, 6, 1, 6, 5,
        ],
    };
    for normal in [[1.0, 0.0, 0.0], [1.0, 1.0, 0.5]] {
        let result = split_with_separator_disc(
            &mesh,
            SeparatorDisc {
                center: [0.0, 0.0, 0.0],
                normal,
                kerf_mm: 0.05,
                radius_mm: 4.0,
            },
        );

        assert!(result.is_ok(), "{result:?}");
        let Ok(result) = result else { return };
        assert!(!result.part_a.indices.is_empty());
        assert!(!result.part_b.indices.is_empty());
        assert_eq!(result.part_a.indices.len() % 3, 0);
        assert_eq!(result.part_b.indices.len() % 3, 0);
        let normal = DVec3::from_array(normal).normalize();
        let positive_min = result
            .part_a
            .positions
            .iter()
            .map(|&position| DVec3::from_array(position).dot(normal))
            .fold(f64::INFINITY, f64::min);
        let negative_max = result
            .part_b
            .positions
            .iter()
            .map(|&position| DVec3::from_array(position).dot(normal))
            .fold(f64::NEG_INFINITY, f64::max);
        assert!((positive_min - 0.025).abs() < 1.0e-8, "{positive_min}");
        assert!((negative_max + 0.025).abs() < 1.0e-8, "{negative_max}");
    }
}

#[test]
fn operator_kerf_and_disc_bounds_remain_valid_and_precise() {
    let mut mesh = empty_mesh();
    append_box(&mut mesh, [-1.0; 3], [1.0; 3]);
    let prepared = prepare_robust_solid(&mesh).expect("prepared box");
    for (kerf_mm, radius_mm) in [(0.01, 4.0), (1.0, 60.0)] {
        let disc = SeparatorDisc {
            center: [0.0; 3],
            normal: [1.0, 0.0, 0.0],
            kerf_mm,
            radius_mm,
        };

        let split = prepared
            .split_with_separator_disc(&identity_transform(), disc)
            .expect("operator parameter bound must split");

        assert!((split_gap(&split, disc.center, disc.normal) - kerf_mm).abs() < 1.0e-8);
    }
}

#[allow(clippy::cast_possible_truncation, clippy::expect_used)]
#[test]
fn normalizing_a_quantized_mesh_keeps_a_closed_component() {
    let source = Manifold::cube(2.0, 2.0, 2.0, true).translate(0.191_620_149, 0.0, 0.0);
    let (positions, property_count, indices) = source.to_mesh_f64();
    let positions = positions
        .chunks_exact(property_count)
        .map(|vertex| {
            [
                f64::from(vertex[0] as f32),
                f64::from(vertex[1] as f32),
                f64::from(vertex[2] as f32),
            ]
        })
        .collect();

    let normalized = normalize_closed_mesh(&RobustMesh { positions, indices })
        .expect("quantized manifold remains closed");
    assert!(!normalized.positions.is_empty());
    assert!(!normalized.indices.is_empty());
}
