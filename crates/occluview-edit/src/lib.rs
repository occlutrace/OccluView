//! `occluview-edit` — mesh-editing façade over the product-neutral
//! `occluview-mesh-edit` kernel.
//!
//! The kernel works on its own flat buffers and knows nothing about the core
//! scene model; this façade adapts a `occluview-core`
//! [`Mesh`](occluview_core::Mesh) to those buffers, runs the requested
//! operation, and rebuilds a core mesh. It also owns the world-space
//! bridge-split adapter and the scene section assembly, which are the only
//! parts of the model that call the kernel. Keeping them here leaves
//! `occluview-core` a pure data model with no kernel dependency.
//!
//! ## Invariants
//!
//! - **Panic-free.** Every public function returns a `Result` or is total.
//! - **`Send + Sync`.** All public types are shareable across threads; the
//!   app's edit worker relies on this for the prepared bridge-split source.

#![cfg_attr(not(test), deny(clippy::panic))]
#![forbid(unsafe_code)]
// In tests we relax strict runtime lints: `unwrap`/`expect`/`float_cmp`/`as`
// casts are legitimate test conveniences.
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

mod bridge_split;
mod edit;
mod section;

pub use bridge_split::{
    bridge_split_mesh_in_world, bridge_split_prepared_mesh_in_world, normalize_bridge_split_input,
    prepare_bridge_split_source, CoreBridgeSplitError, CoreBridgeSplitResult,
    PreparedBridgeSplitSource,
};
pub use edit::{
    component_at_triangle_in_mesh, crop_mesh_to_selected_faces, delete_selected_faces_in_mesh,
    fill_holes_in_mesh, fill_selected_holes_in_mesh, invert_mesh_orientation,
    mesh_edit_buffers_from_mesh, mesh_from_edit_buffers_like, mesh_from_sculpt_session_like,
    repair_mesh_in_mesh, selected_connected_components_in_mesh, CoreMeshEditResult,
    CoreMeshRepairResult, SculptSessionBuffers,
};
pub use occluview_mesh_edit::{
    BridgeSplitError, BridgeSplitReport, BridgeSplitRequest, EditVertex, FaceSelection,
    MeshEditBuffers, MeshEditError, MeshEditOptions, MeshEditReport, MeshEditWarning, MeshTopology,
    RepairOptions, RepairReport, CLOSE_HOLES_EDGE_CEILING,
};
pub use section::{LayerSection, SceneSection, SectionCache};

/// Scene section assembly over `occluview-core`'s scene graph.
pub mod scene {
    pub use crate::section::{LayerSection, SceneSection, SectionCache};
    pub use occluview_mesh_edit::{SectionPlane, SectionPolyline, SectionResult};
}
