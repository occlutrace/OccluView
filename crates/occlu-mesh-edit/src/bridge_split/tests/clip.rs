use super::{
    closed_cube, exploded_cube_with_payload_seams, exploded_cube_with_uniform_payload, request,
};
use crate::bridge_split::clip::clip_bridge_open;
use crate::{BridgeSplitError, BridgeSplitRequest, MeshEditBuffers};
use glam::Vec3;

fn axis_bounds(mesh: &MeshEditBuffers, axis: usize) -> (f32, f32) {
    mesh.vertices
        .iter()
        .map(|vertex| vertex.position[axis])
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(min, max), value| {
            (min.min(value), max.max(value))
        })
}

fn request_with_kerf(kerf_mm: f32) -> BridgeSplitRequest {
    BridgeSplitRequest {
        kerf_mm,
        ..request()
    }
}

#[test]
fn cube_is_clipped_to_two_kerf_boundaries() {
    let result = clip_bridge_open(&closed_cube(), request()).expect("cube clips");
    let positive_bounds = axis_bounds(&result.part_a, 0);
    let negative_bounds = axis_bounds(&result.part_b, 0);

    assert!((positive_bounds.0 - 0.025).abs() <= 1.0e-6);
    assert!((positive_bounds.1 - 1.0).abs() <= f32::EPSILON);
    assert!((negative_bounds.0 + 1.0).abs() <= f32::EPSILON);
    assert!((negative_bounds.1 + 0.025).abs() <= 1.0e-6);
    assert!(((positive_bounds.0 - negative_bounds.1) - 0.05).abs() <= 2.0e-6);
}

#[test]
fn minimum_and_maximum_ui_kerfs_remain_distinct() {
    for kerf_mm in [0.01, 1.0] {
        let result = clip_bridge_open(&closed_cube(), request_with_kerf(kerf_mm))
            .expect("supported kerf clips");
        let (part_a_min, _) = axis_bounds(&result.part_a, 0);
        let (_, part_b_max) = axis_bounds(&result.part_b, 0);
        assert!(((part_a_min - part_b_max) - kerf_mm).abs() <= 2.0e-6);
    }
}

#[test]
fn oblique_plane_through_source_vertices_is_stable() {
    let mut request = request();
    request.normal = Vec3::new(1.0, -1.0, 0.0);

    let result = clip_bridge_open(&closed_cube(), request).expect("oblique cut clips");
    assert!(!result.part_a.indices.is_empty());
    assert!(!result.part_b.indices.is_empty());
    assert!(result.part_a_cut_edges.len() >= 3);
    assert!(result.part_b_cut_edges.len() >= 3);
}

#[test]
fn near_face_cut_does_not_emit_degenerate_triangles() {
    let mut request = request_with_kerf(0.01);
    request.center.x = 0.994;
    let result = clip_bridge_open(&closed_cube(), request).expect("thin side remains valid");

    for mesh in [&result.part_a, &result.part_b] {
        for face in mesh.indices.chunks_exact(3) {
            assert_ne!(face[0], face[1]);
            assert_ne!(face[1], face[2]);
            assert_ne!(face[2], face[0]);
            let a = Vec3::from_array(mesh.vertices[face[0] as usize].position);
            let b = Vec3::from_array(mesh.vertices[face[1] as usize].position);
            let c = Vec3::from_array(mesh.vertices[face[2] as usize].position);
            assert!((b - a).cross(c - a).length_squared() > 0.0);
        }
    }
}

#[test]
fn large_coordinates_do_not_break_classification_or_reported_kerf() {
    let mut mesh = closed_cube();
    for vertex in &mut mesh.vertices {
        vertex.position[0] += 1_000_000.0;
    }
    let mut request = request();
    request.center.x = 1_000_000.0;
    let result = clip_bridge_open(&mesh, request).expect("relative f64 clipping succeeds");
    let (part_a_min, _) = axis_bounds(&result.part_a, 0);
    let (_, part_b_max) = axis_bounds(&result.part_b, 0);

    assert!(!result.part_a.indices.is_empty());
    assert!(!result.part_b.indices.is_empty());
    assert!((result.report.kerf_mm - 0.05).abs() <= f32::EPSILON);
    assert!((part_a_min - part_b_max - 0.05).abs() <= 0.125);
}

#[test]
fn clipping_is_deterministic_and_does_not_mutate_source() {
    let source = exploded_cube_with_payload_seams();
    let original = source.clone();
    let first = clip_bridge_open(&source, request()).expect("first run");
    let second = clip_bridge_open(&source, request()).expect("second run");

    assert_eq!(source, original);
    assert_eq!(first, second);
}

#[test]
fn generated_vertices_interpolate_color_and_uv() {
    let mut mesh = closed_cube();
    for vertex in &mut mesh.vertices {
        let t = (vertex.position[0] + 1.0) * 0.5;
        vertex.color = [if vertex.position[0] < 0.0 { 0 } else { 200 }, 20, 40, 255];
        vertex.uv = [t, 1.0 - t];
    }
    let result = clip_bridge_open(&mesh, request()).expect("colored cube clips");
    let boundary = result
        .part_a
        .vertices
        .iter()
        .find(|vertex| (vertex.position[0] - 0.025).abs() <= 1.0e-6)
        .expect("generated boundary vertex");

    assert!((i16::from(boundary.color[0]) - 103).abs() <= 1);
    assert!((boundary.uv[0] - 0.5125).abs() <= 1.0e-5);
    assert!((boundary.uv[1] - 0.4875).abs() <= 1.0e-5);
}

#[test]
fn uniform_soup_faces_share_exact_cut_vertices() {
    let result = clip_bridge_open(&exploded_cube_with_uniform_payload(), request())
        .expect("uniform soup clips");
    let boundary_count = result
        .part_a
        .vertices
        .iter()
        .filter(|vertex| (vertex.position[0] - 0.025).abs() <= 1.0e-6)
        .count();

    assert_eq!(boundary_count, 8, "shared source edges must share cut ids");
}

#[test]
fn payload_seams_remain_distinct_on_the_cut_boundary() {
    let result = clip_bridge_open(&exploded_cube_with_payload_seams(), request())
        .expect("seamed soup clips");
    let boundary: Vec<_> = result
        .part_a
        .vertices
        .iter()
        .filter(|vertex| (vertex.position[0] - 0.025).abs() <= 1.0e-6)
        .collect();
    let has_position_with_distinct_payloads =
        boundary.iter().enumerate().any(|(first_index, first)| {
            boundary.iter().skip(first_index + 1).any(|second| {
                first.position == second.position
                    && (first.color != second.color || first.uv != second.uv)
            })
        });

    assert!(has_position_with_distinct_payloads);
}

#[test]
fn normal_only_seams_remain_distinct_on_the_cut_boundary() {
    let mut mesh = exploded_cube_with_uniform_payload();
    for (corner, vertex) in mesh.vertices.iter_mut().enumerate() {
        vertex.normal = if (corner / 3) % 2 == 0 {
            [0.0, 1.0, 0.0]
        } else {
            [0.0, 0.0, 1.0]
        };
    }
    let result = clip_bridge_open(&mesh, request()).expect("normal-seamed soup clips");
    let boundary: Vec<_> = result
        .part_a
        .vertices
        .iter()
        .filter(|vertex| (vertex.position[0] - 0.025).abs() <= 1.0e-6)
        .collect();

    assert!(boundary.iter().enumerate().any(|(first_index, first)| {
        boundary
            .iter()
            .skip(first_index + 1)
            .any(|second| first.position == second.position && first.normal != second.normal)
    }));
}

#[test]
fn near_plane_source_vertex_is_snapped_to_one_rim_identity() {
    let mut mesh = closed_cube();
    mesh.vertices[1].position[0] = 0.025_001;
    let result = clip_bridge_open(&mesh, request()).expect("near-plane vertex clips");
    let snapped: Vec<_> = result
        .part_a
        .vertices
        .iter()
        .filter(|vertex| (vertex.position[0] - 0.025).abs() <= 1.0e-6)
        .collect();

    assert!(!snapped.is_empty());
    assert!(result.part_a_cut_edges.len() >= 3);
}

#[test]
fn miss_and_tangent_contact_are_typed_failures() {
    let mut miss = request();
    miss.center.x = 3.0;
    assert!(matches!(
        clip_bridge_open(&closed_cube(), miss),
        Err(BridgeSplitError::NoIntersection)
    ));

    let mut tangent = request();
    tangent.center.x = 1.025;
    assert!(matches!(
        clip_bridge_open(&closed_cube(), tangent),
        Err(BridgeSplitError::TangentContact)
    ));
}

#[test]
fn finite_disc_uses_slab_polygon_extent_not_triangle_centroids() {
    let mut mesh = closed_cube();
    for vertex in &mut mesh.vertices {
        vertex.position[1] *= 10.0;
        vertex.position[2] *= 10.0;
    }
    let mut limited = request();
    limited.disc_radius_mm = 12.0;
    limited.max_disc_radius_mm = 12.0;

    assert!(matches!(
        clip_bridge_open(&mesh, limited),
        Err(BridgeSplitError::DiscLimitExceeded {
            required_radius_mm,
            max_radius_mm: 12.0,
        }) if required_radius_mm > 14.0
    ));
}

#[test]
fn selected_disc_must_span_the_complete_kerf_cross_section() {
    let mut mesh = closed_cube();
    for vertex in &mut mesh.vertices {
        vertex.position[1] *= 10.0;
        vertex.position[2] *= 10.0;
    }
    let mut too_small = request();
    too_small.disc_radius_mm = 12.0;

    let required_radius = match clip_bridge_open(&mesh, too_small) {
        Err(BridgeSplitError::DiscTooSmall {
            disc_radius_mm,
            required_radius_mm,
        }) => {
            assert_eq!(disc_radius_mm.to_bits(), 12.0_f32.to_bits());
            assert!(required_radius_mm > 14.0);
            required_radius_mm
        }
        other => panic!("expected an actionable disc-size failure, got {other:?}"),
    };

    let mut complete_disc = too_small;
    complete_disc.disc_radius_mm = required_radius + 0.1;
    let result = clip_bridge_open(&mesh, complete_disc).expect("complete disc spans the cut");
    assert_eq!(
        result.report.disc_radius_mm.to_bits(),
        complete_disc.disc_radius_mm.to_bits()
    );
    assert!(result.report.required_disc_radius_mm <= result.report.disc_radius_mm);
}

#[test]
fn larger_complete_discs_leave_the_two_output_parts_unchanged() {
    let mesh = closed_cube();
    let mut compact = request();
    compact.disc_radius_mm = 2.0;
    let compact = clip_bridge_open(&mesh, compact).expect("2 mm disc spans a 2 mm cube section");

    let mut large = request();
    large.disc_radius_mm = 40.0;
    let large = clip_bridge_open(&mesh, large).expect("larger disc also spans the section");

    assert_eq!(compact.part_a, large.part_a);
    assert_eq!(compact.part_b, large.part_b);
    assert_eq!(compact.part_a_cut_edges, large.part_a_cut_edges);
    assert_eq!(compact.part_b_cut_edges, large.part_b_cut_edges);
    assert_eq!(
        compact.report.required_disc_radius_mm,
        large.report.required_disc_radius_mm
    );
}
