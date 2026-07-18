//! Product-neutral mesh edit foundations.
//!
//! This crate owns pure geometry data movement and validation used by future
//! edit kernels. It intentionally knows nothing about OccluView UI, renderer,
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
mod brush;
mod brush_index;
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
mod error;
mod holes;
mod holes_cleanup;
mod holes_gate;
mod holes_walk;
mod normals;
mod orientation;
mod pinch;
mod repair;
mod section;
mod topology;
mod topology_analysis;
mod types;
mod validate;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod brush_tests;

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
pub use brush::{smooth_selected_faces, BrushMode, BrushSession, BrushStroke, BrushStrokeOutcome};
pub use component_pick::component_at_triangle;
pub use components::selected_connected_components;
pub use delete_crop::{crop_to_selected_faces, delete_selected_faces};
pub use error::{BridgeSplitError, MeshEditError};
pub use holes::{fill_holes, fill_selected_holes};
pub use normals::recompute_all_normals;
pub use orientation::invert_orientation;
pub use repair::{repair_mesh, RepairOptions, RepairReport, RepairResult};
pub use section::{plane_section, SectionError, SectionPlane, SectionPolyline, SectionResult};
pub use types::{
    EditVertex, FaceSelection, GeneratedVertexPolicy, MeshEditAttributePolicy, MeshEditBuffers,
    MeshEditOptions, MeshEditReport, MeshEditResult, MeshEditWarning, MeshTopology,
};
pub use validate::{
    validate_face_edit_buffers, validate_mesh_edit_options,
    validate_selection_against_triangle_count, validate_triangle_mesh_data,
};
