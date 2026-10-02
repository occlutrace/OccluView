//! Mesh editing for `occluview-core` meshes.
//!
//! This crate owns pure geometry editing and validation plus the adapter that
//! runs those operations on the core data model: buffer conversion, the
//! world-space bridge split, the scene section assembly, and the optional
//! native CSG fallback. It intentionally knows nothing about the UI, renderer,
//! shell integration, HPS decoding, or product-specific CAD state.

#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::panic))]
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::float_cmp,
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss,
        clippy::cast_lossless,
        clippy::cast_possible_wrap,
    )
)]

mod adjacency;
mod attributes;
mod bridge_split;
mod cap_delaunay;
mod cap_fair;
mod cap_fit;
mod cap_guard;
mod cap_lawson;
mod cap_minweight;
mod cap_refine;
mod cap_support;
mod component_pick;
mod components;
mod delete_crop;
mod edit;
mod error;
mod holes;
mod holes_cleanup;
mod holes_gate;
mod holes_walk;
mod normals;
mod numeric;
mod orientation;
mod pinch;
mod repair;
/// Robust finite-disc CSG fallback for closed triangle meshes.
///
/// This module owns the native Manifold dependency directly, so enabling the
/// `robust-csg` feature builds the C++ kernel as part of this crate.
#[cfg(feature = "robust-csg")]
pub mod robust;
mod scene_section;
mod section;
mod topology;
mod topology_analysis;
mod types;
mod validate;
mod world_bridge_split;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod holes_matrix_tests;

#[cfg(test)]
mod holes_socket_tests;

#[cfg(test)]
mod holes_soup_tests;

#[cfg(test)]
mod holes_tests;

pub use attributes::{copy_surviving_vertices, remap_triangle_indices};
pub use bridge_split::{
    split_bridge, split_bridge_surface, validate_bridge_split, validate_bridge_split_part,
    validate_bridge_split_request, BridgeSplitReport, BridgeSplitRequest, BridgeSplitResult,
    SurfaceSplitResult,
};
pub use component_pick::component_at_triangle;
pub use components::selected_connected_components;
pub use delete_crop::{crop_to_selected_faces, delete_selected_faces};
pub use edit::{
    component_at_triangle_in_mesh, crop_mesh_to_selected_faces, delete_selected_faces_in_mesh,
    fill_holes_in_mesh, fill_selected_holes_in_mesh, invert_mesh_orientation,
    mesh_edit_buffers_from_mesh, mesh_from_edit_buffers_like, mesh_from_sculpt_session_like,
    repair_mesh_in_mesh, selected_connected_components_in_mesh, CoreMeshEditResult,
    CoreMeshRepairResult, SculptSessionBuffers,
};
pub use error::{BridgeSplitError, MeshEditError};
pub use holes::{fill_holes, fill_selected_holes, CLOSE_HOLES_EDGE_CEILING};
pub use normals::recompute_all_normals;
pub use occluview_geometry::{coincident_position_key, COINCIDENT_POSITION_EPS_MM};
pub use orientation::invert_orientation;
pub use repair::{repair_mesh, RepairOptions, RepairReport, RepairResult};
pub use scene_section::{LayerSection, SceneSection, SectionCache};
pub use section::{plane_section, SectionError, SectionPlane, SectionPolyline, SectionResult};
pub use types::{
    EditVertex, FaceSelection, GeneratedVertexPolicy, MeshEditAttributePolicy, MeshEditBuffers,
    MeshEditOptions, MeshEditReport, MeshEditResult, MeshEditWarning, MeshTopology,
};
pub use validate::{
    validate_face_edit_buffers, validate_mesh_edit_options,
    validate_selection_against_triangle_count, validate_triangle_mesh_data,
};
pub use world_bridge_split::{
    bridge_split_mesh_in_world, bridge_split_prepared_mesh_in_world, normalize_bridge_split_input,
    prepare_bridge_split_source, CoreBridgeSplitError, CoreBridgeSplitResult,
    PreparedBridgeSplitSource,
};

/// Scene section assembly over `occluview-core`'s scene graph.
pub mod scene {
    pub use crate::scene_section::{LayerSection, SceneSection, SectionCache};
    pub use crate::section::{SectionPlane, SectionPolyline, SectionResult};
}
