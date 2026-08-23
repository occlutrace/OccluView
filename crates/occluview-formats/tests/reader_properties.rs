//! Properties every reader has to hold, over inputs that actually parse.
//!
//! The fuzz targets look for crashes; these look for a mesh that is internally
//! inconsistent, which a fuzzer has no oracle for. An index past the end of the
//! vertex array is not a crash in the reader -- it is a crash, or a silently
//! wrong picture, somewhere downstream: the GPU upload, the BVH build, the
//! duplicate-normal pass, all of which trust what the reader returned.
//!
//! The generators build well-formed containers with arbitrary contents, so the
//! success path is exercised rather than the reject-everything path a stream of
//! random bytes takes.

// Bit-for-bit is the point of the determinism property: two reads of the
// same bytes must produce the same floats, not merely close ones.
#![allow(clippy::expect_used, clippy::float_cmp)]

use occluview_formats::dispatch::dispatch_by_extension;
use proptest::prelude::*;

/// A binary STL of `triangles` triangles with the given coordinates.
fn binary_stl(coordinates: &[f32]) -> Vec<u8> {
    let triangles = coordinates.len() / 12;
    let mut bytes = vec![0u8; 80];
    bytes.extend_from_slice(&u32::try_from(triangles).unwrap_or(0).to_le_bytes());
    for triangle in coordinates.chunks_exact(12) {
        for value in triangle {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes.extend_from_slice(&[0, 0]);
    }
    bytes
}

/// An OBJ with `vertices` positions and faces built from `face_indices`, which
/// are allowed to point anywhere.
fn obj(vertices: &[f32], face_indices: &[u32]) -> String {
    use std::fmt::Write as _;
    let mut text = String::new();
    for position in vertices.chunks_exact(3) {
        let _ = writeln!(text, "v {} {} {}", position[0], position[1], position[2]);
    }
    for face in face_indices.chunks_exact(3) {
        let _ = writeln!(text, "f {} {} {}", face[0], face[1], face[2]);
    }
    text
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    /// A binary STL always parses, and what comes back has to be consistent.
    #[test]
    fn a_binary_stl_reads_into_a_mesh_that_indexes_only_its_own_vertices(
        coordinates in proptest::collection::vec(-1.0e3_f32..1.0e3, 12..=120),
    ) {
        let usable = coordinates.len() / 12 * 12;
        let bytes = binary_stl(&coordinates[..usable]);
        let mesh = dispatch_by_extension("stl", &bytes).expect("a well-formed binary STL");

        let vertices = mesh.vertices().len();
        for index in mesh.indices() {
            prop_assert!(
                (*index as usize) < vertices,
                "index {index} against {vertices} vertices"
            );
        }
        prop_assert_eq!(mesh.indices().len() % 3, 0);
        prop_assert_eq!(mesh.triangle_count() * 3, mesh.indices().len());
        for vertex in mesh.vertices() {
            let normal = vertex.normal;
            prop_assert!(
                normal.iter().all(|component| component.is_finite()),
                "a normal came back as {normal:?}"
            );
        }
    }

    /// An OBJ whose faces point anywhere must be refused or clamped, never
    /// returned with an index past the end.
    #[test]
    fn an_obj_with_arbitrary_face_indices_is_refused_or_consistent(
        vertices in proptest::collection::vec(-1.0e3_f32..1.0e3, 3..=60),
        faces in proptest::collection::vec(1_u32..40, 3..=30),
    ) {
        let usable_vertices = vertices.len() / 3 * 3;
        let usable_faces = faces.len() / 3 * 3;
        let text = obj(&vertices[..usable_vertices], &faces[..usable_faces]);

        if let Ok(mesh) = dispatch_by_extension("obj", text.as_bytes()) {
            let count = mesh.vertices().len();
            for index in mesh.indices() {
                prop_assert!(
                    (*index as usize) < count,
                    "index {index} against {count} vertices"
                );
            }
            prop_assert_eq!(mesh.indices().len() % 3, 0);
        }
    }

    /// The same bytes read twice give the same mesh.
    ///
    /// The readers parallelise over triangles, and a parse that depends on
    /// thread scheduling is a scan that changes between opening it and
    /// exporting it.
    #[test]
    fn reading_the_same_bytes_twice_gives_the_same_mesh(
        coordinates in proptest::collection::vec(-1.0e3_f32..1.0e3, 12..=240),
    ) {
        let usable = coordinates.len() / 12 * 12;
        let bytes = binary_stl(&coordinates[..usable]);
        let first = dispatch_by_extension("stl", &bytes).expect("a well-formed binary STL");
        let second = dispatch_by_extension("stl", &bytes).expect("a well-formed binary STL");

        prop_assert_eq!(first.vertices().len(), second.vertices().len());
        prop_assert_eq!(first.indices(), second.indices());
        for (a, b) in first.vertices().iter().zip(second.vertices()) {
            prop_assert_eq!(a.position, b.position);
            prop_assert_eq!(a.normal, b.normal);
        }
    }

    /// And arbitrary bytes under any offered extension must not panic.
    #[test]
    fn arbitrary_bytes_under_any_extension_are_answered_not_survived(
        extension_choice in 0_usize..7,
        bytes in proptest::collection::vec(any::<u8>(), 0..512),
    ) {
        let extension = ["stl", "ply", "obj", "glb", "hps", "dcm", "off"][extension_choice];
        let _ = dispatch_by_extension(extension, &bytes);
    }
}
