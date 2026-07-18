//! `occluview-core` — pure logic for OccluView.
//!
//! This crate is intentionally free of I/O, GPU, and platform (Win32) concerns.
//! It contains the domain data model: units, math, mesh representation, the
//! scene graph, and the camera. Both the renderer and the GUI/CLI/shell build on
//! it.
//!
//! ## Invariants
//!
//! - **Panic-free.** Every public function returns a `Result` or is total. There
//!   is no `unwrap`/`expect`/`panic!` in this crate (clippy-enforced).
//! - **`Send + Sync`.** All public types are shareable across threads; the
//!   renderer and the file loaders rely on this.
//! - **Millimeter units** internally ([`units::Millimeters`]).
//! - **Right-handed Y-up** coordinate frame ([`frame`]).
//!
//! The crate is organized as follows; each module re-exports its public surface
//! from here so callers can `use occluview_core::Mesh` etc.

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

pub mod bbox;
pub mod camera;
pub mod error;
pub mod frame;
pub mod jump_list;
pub mod mesh;
pub mod recent_files;
pub mod scale_bar;
pub mod scene;
pub mod units;

pub use bbox::Aabb;
pub use camera::{
    orbit_delta_from_pointer_motion, zoom_factor_from_scroll, Camera, CameraAxisView, CameraPreset,
    CameraProjection, CAD_ORBIT_DRAG_GAIN, CAD_ZOOM_SCROLL_SENSITIVITY,
};
pub use error::CoreError;
pub use jump_list::JumpListItem;
pub use mesh::{
    bridge_split_mesh_in_world, bridge_split_prepared_mesh_in_world, component_at_triangle_in_mesh,
    crop_mesh_to_selected_faces, delete_selected_faces_in_mesh, fill_holes_in_mesh,
    fill_selected_holes_in_mesh, invert_mesh_orientation, mesh_edit_buffers_from_mesh,
    mesh_from_edit_buffers_like, normalize_bridge_split_input, prepare_bridge_split_source,
    repair_mesh_in_mesh, selected_connected_components_in_mesh, CoreBridgeSplitError,
    CoreBridgeSplitResult, CoreMeshEditResult, CoreMeshRepairResult, Mesh, MeshBuilder, MeshKind,
    MeshTexture, PreparedBridgeSplitSource, PrincipalFrame, Vertex,
};
pub use occlu_mesh_edit::{
    BridgeSplitError, BridgeSplitReport, BridgeSplitRequest, BrushMode, BrushSession, BrushStroke,
    BrushStrokeOutcome, FaceSelection, MeshEditOptions, MeshEditReport, MeshEditWarning,
    RepairOptions, RepairReport,
};
pub use recent_files::{RecentEntry, RecentFiles};
pub use scale_bar::ScaleBar;
pub use scene::{Scene, SceneMesh, SceneMeshId, ScenePickHit, DEFAULT_UNTEXTURED_MESH_TINT};
pub use units::Millimeters;
