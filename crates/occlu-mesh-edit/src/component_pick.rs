//! Pick the connected component (one "object") under a triangle.
//!
//! A multi-part prosthesis saved in a CAD tool as one scene and exported to a
//! single STL arrives as several disjoint solids fused into one triangle SOUP
//! (every triangle owns private, unshared vertices). "Which object did I click?"
//! is a connected-component query over the mesh's TRUE topology, so — exactly as
//! [`selected_connected_components`] does for Separate — the soup is welded back
//! to shared topology first (see [`weld_soup_topology`]). Without that weld every
//! triangle is its own component and a click would select a single facet, never
//! the whole object (the 317k-confetti failure mode).

use super::{selected_connected_components, validate_face_edit_buffers, FaceSelection};
use super::{MeshEditBuffers, MeshEditError};

/// Return the triangle indices of the connected component (object) that owns
/// `triangle_index`, in ascending order.
///
/// Connectivity runs on welded topology (STL soup is recovered exactly — see
/// [`selected_connected_components`]), so the returned indices span the whole
/// object the picked facet belongs to, not just the one facet. Triangle order
/// and count are preserved by the weld, so the indices address the CALLER's
/// original buffers directly. Deterministic: the underlying grouping is
/// sort-based and every component's member list is ascending.
///
/// Returns `Ok(None)` when there is nothing to pick — `triangle_index` is out of
/// range, or the mesh has no triangles (a point cloud / faceless mesh is instead
/// rejected up front by validation). Never panics on degenerate or NaN geometry.
///
/// # Errors
/// Returns [`MeshEditError::UnsupportedPointCloud`] for point-cloud topology and
/// [`MeshEditError::MalformedMesh`] for malformed triangle data — the same
/// contract as the sibling kernels.
pub fn component_at_triangle(
    mesh: &MeshEditBuffers,
    triangle_index: usize,
) -> Result<Option<Vec<usize>>, MeshEditError> {
    validate_face_edit_buffers(mesh.topology, &mesh.vertices, &mesh.indices)?;

    let triangle_count = mesh.triangle_count();
    if triangle_index >= triangle_count {
        return Ok(None);
    }

    // Reuse the exact-bit weld + connectivity kernel: a full-true selection makes
    // every triangle a candidate, so `selected_connected_components` returns the
    // mesh's real objects. Each member list is ascending, so the picked triangle
    // is located by binary search rather than a linear scan.
    let selection = FaceSelection::new(vec![true; triangle_count]);
    let components = selected_connected_components(mesh, &selection)?;
    Ok(components
        .into_iter()
        .find(|component| component.binary_search(&triangle_index).is_ok()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{EditVertex, MeshTopology};

    /// Two independent quads (four triangles each is overkill; two each is
    /// enough) welded, then STL-exploded to soup: object A near the origin,
    /// object B translated far along +x so the two never share a corner.
    fn two_object_soup() -> MeshEditBuffers {
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        // Each object is a quad (two edge-connected triangles) emitted as soup:
        // six private vertices per object, sequential indices.
        for base_x in [0.0_f32, 100.0] {
            let quad = [
                [base_x, 0.0, 0.0],
                [base_x + 1.0, 0.0, 0.0],
                [base_x + 1.0, 1.0, 0.0],
                [base_x, 1.0, 0.0],
            ];
            // Triangle (0,1,2) and (0,2,3) share edge 0-2 -> one component.
            for corner in [quad[0], quad[1], quad[2], quad[0], quad[2], quad[3]] {
                indices.push(vertices.len() as u32);
                vertices.push(EditVertex::at(corner));
            }
        }
        MeshEditBuffers {
            vertices,
            indices,
            topology: MeshTopology::TriangleMesh,
        }
    }

    #[test]
    fn picks_the_whole_object_not_confetti_and_not_the_neighbour() {
        // Object A owns soup triangles 0,1; object B owns 2,3. Clicking any facet
        // of A returns exactly A's two triangles — never one confetti facet, and
        // never a triangle of B.
        let mesh = two_object_soup();
        for picked in [0_usize, 1] {
            let component = component_at_triangle(&mesh, picked)
                .expect("valid mesh")
                .expect("picked triangle belongs to a component");
            assert_eq!(component, vec![0, 1], "click on A selects all of A");
        }
        for picked in [2_usize, 3] {
            let component = component_at_triangle(&mesh, picked)
                .expect("valid mesh")
                .expect("picked triangle belongs to a component");
            assert_eq!(component, vec![2, 3], "click on B selects all of B");
        }
    }

    #[test]
    fn single_object_returns_every_triangle() {
        // A lone welded strip exploded to soup is ONE object: any pick returns
        // the whole strip.
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        let quad = [
            [0.0_f32, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ];
        for corner in [quad[0], quad[1], quad[2], quad[0], quad[2], quad[3]] {
            indices.push(vertices.len() as u32);
            vertices.push(EditVertex::at(corner));
        }
        let mesh = MeshEditBuffers {
            vertices,
            indices,
            topology: MeshTopology::TriangleMesh,
        };
        let component = component_at_triangle(&mesh, 1)
            .expect("valid mesh")
            .expect("component");
        assert_eq!(component, vec![0, 1]);
    }

    #[test]
    fn out_of_range_triangle_is_a_none_noop() {
        let mesh = two_object_soup();
        assert_eq!(component_at_triangle(&mesh, 4), Ok(None));
        assert_eq!(component_at_triangle(&mesh, usize::MAX), Ok(None));
    }

    #[test]
    fn faceless_mesh_is_a_none_noop_not_a_panic() {
        // A triangle-topology mesh with no indices (points only, e.g. a decimated
        // scan reduced to vertices) has no components: an honest None, no panic.
        let mesh = MeshEditBuffers {
            vertices: vec![EditVertex::at([0.0, 0.0, 0.0])],
            indices: Vec::new(),
            topology: MeshTopology::TriangleMesh,
        };
        assert_eq!(component_at_triangle(&mesh, 0), Ok(None));
    }

    #[test]
    fn point_cloud_is_rejected_like_the_sibling_kernels() {
        let mesh = MeshEditBuffers {
            vertices: vec![EditVertex::at([0.0, 0.0, 0.0])],
            indices: Vec::new(),
            topology: MeshTopology::PointCloud,
        };
        assert_eq!(
            component_at_triangle(&mesh, 0),
            Err(MeshEditError::UnsupportedPointCloud)
        );
    }

    #[test]
    fn is_deterministic_across_repeated_calls() {
        let mesh = two_object_soup();
        let first = component_at_triangle(&mesh, 3)
            .expect("valid")
            .expect("some");
        let second = component_at_triangle(&mesh, 3)
            .expect("valid")
            .expect("some");
        assert_eq!(first, second);
    }

    #[test]
    fn nan_and_degenerate_triangles_do_not_panic() {
        // One object A plus a NaN-carrying triangle: picking A must still return
        // exactly A, and picking the NaN facet must return that facet's own
        // component without panicking.
        let nan = f32::NAN;
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        let quad = [
            [0.0_f32, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ];
        for corner in [quad[0], quad[1], quad[2], quad[0], quad[2], quad[3]] {
            indices.push(vertices.len() as u32);
            vertices.push(EditVertex::at(corner));
        }
        // A separate, isolated triangle with a NaN coordinate.
        for corner in [[nan, 0.0, 0.0], [nan, 1.0, 0.0], [nan, 0.0, 1.0]] {
            indices.push(vertices.len() as u32);
            vertices.push(EditVertex::at(corner));
        }
        let mesh = MeshEditBuffers {
            vertices,
            indices,
            topology: MeshTopology::TriangleMesh,
        };
        assert_eq!(
            component_at_triangle(&mesh, 0)
                .expect("valid")
                .expect("some"),
            vec![0, 1],
            "the clean object is unaffected by the NaN facet"
        );
        let nan_component = component_at_triangle(&mesh, 2)
            .expect("valid")
            .expect("some");
        assert!(
            nan_component.contains(&2),
            "the NaN facet resolves to a component without panicking"
        );
    }

    /// A ~500k-triangle grid emitted as STL soup: every triangle's corners are
    /// fresh vertices placed at the exact grid-point positions, so the exact-bit
    /// weld recovers the whole grid as ONE connected object.
    fn soup_grid(cells: u32) -> MeshEditBuffers {
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        let point = |x: u32, y: u32| [x as f32, y as f32, 0.0];
        for iy in 0..cells {
            for ix in 0..cells {
                let corners = [
                    point(ix, iy),
                    point(ix + 1, iy),
                    point(ix + 1, iy + 1),
                    point(ix, iy),
                    point(ix + 1, iy + 1),
                    point(ix, iy + 1),
                ];
                for corner in corners {
                    indices.push(vertices.len() as u32);
                    vertices.push(EditVertex::at(corner));
                }
            }
        }
        MeshEditBuffers {
            vertices,
            indices,
            topology: MeshTopology::TriangleMesh,
        }
    }

    // Intentional diagnostic: the perf smoke prints its measured wall time to the
    // test log (the crate otherwise denies stray prints).
    #[allow(clippy::print_stderr)]
    #[test]
    fn perf_component_pick_on_a_half_million_triangle_soup_stays_interactive() {
        // 500 x 500 cells -> 500_000 triangles, 1_500_000 soup vertices. Picking
        // any facet must resolve the whole welded object and stay well under an
        // interactive budget (a LOOSE bound: the box is shared under load, this
        // guards against an O(N^2) regression, not a precise benchmark).
        let cells = 500;
        let mesh = soup_grid(cells);
        let triangle_count = mesh.triangle_count();
        assert_eq!(triangle_count, (cells * cells * 2) as usize);

        let started = std::time::Instant::now();
        let component = component_at_triangle(&mesh, triangle_count / 2)
            .expect("valid mesh")
            .expect("picked triangle belongs to the one welded object");
        let elapsed = started.elapsed();
        eprintln!(
            "perf: component_at_triangle over {triangle_count} soup triangles \
             ({} verts) returned {} triangles in {elapsed:?}",
            mesh.vertices.len(),
            component.len(),
        );

        assert_eq!(
            component.len(),
            triangle_count,
            "the welded grid is one object, so the pick returns every triangle"
        );
        assert!(
            elapsed < std::time::Duration::from_secs(10),
            "component pick on 500k soup triangles must stay bounded, took {elapsed:?}"
        );
    }
}
