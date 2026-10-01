//! `occluview-render` - the wgpu renderer.
//!
//! Two consumers share this code: the live GUI (`occluview-app`) and the
//! offscreen thumbnail renderer (`occluview-thumbnail`). They share the camera
//! and the vertex layout, but the live path draws with multisampling into the
//! window while the offscreen path draws without it into an `Rgba8Unorm`
//! viewport-sized target, so the same mesh does not rasterize identically.
//! What can also differ is the mesh: above the fidelity cutoffs in
//! `occluview-thumbnail`, Explorer gets a decimated preview mesh through the
//! offscreen path.
//!
//! ## Layout
//!
//! - [`camera`] - the GPU-side camera uniform (matches the WGSL `Camera`
//!   struct byte-for-byte).
//! - [`clipping`] - the clip plane and the cut-view spec the cut passes share.
//! - [`contact_texture`] - the packed contact field a layer paints by (group 2),
//!   with its ramp carried in the per-mesh uniform.
//! - [`cut_camera`] - section and slice camera builders.
//! - [`error`] - the renderer error type.
//! - [`gpu`] - GPU mesh upload (vertex/index buffers) from `occluview_core::Mesh`.
//! - [`mesh_uniform`] - the per-mesh uniform that carries the contact ramp.
//! - [`offscreen`] - headless render-to-texture (thumbnails, golden tests).
//! - [`pipeline`] - render pipeline creation (device + shader + layout).
//! - [`sculpt_cursor`] - brush and tool uniforms plus the CPU field-parity call.
//! - [`texture`] - GPU texture upload, fallback and box-down for oversized atlases.
//!
//! ## Status
//!
//! v1 pipeline: studio lighting, vertex colors, depth-tested indexed draws,
//! and point-cloud draws. WGSL source in `shaders/mesh.wgsl`.

#![forbid(unsafe_code)]
// GPU buffer/texture sizes are usize->u64/u32 by nature; allow once at the
// crate root rather than per-call-site.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]
#![cfg_attr(test, allow(clippy::float_cmp))]

pub mod camera;
pub mod clipping;
pub mod contact_texture;
pub mod cut_camera;
pub mod error;
pub mod gpu;
pub mod mesh_uniform;
pub mod offscreen;
pub mod pipeline;
pub mod sculpt_cursor;
pub mod texture;

pub use camera::{camera_ortho_proj_matrix, camera_view_matrix, GpuCamera};
pub use clipping::{ClipPlane, CutViewSpec};
pub use contact_texture::{ContactFieldTexels, GpuContactMaterial, FIELD_FAR_SENTINEL_MM};
pub use cut_camera::{
    cut_view_camera, cut_view_camera_focused, cut_view_camera_focused_with_up, slice_view_basis,
    slice_view_basis_with_up,
};
pub use error::RenderError;
pub use gpu::GpuMesh;
pub use mesh_uniform::{GpuMeshUniform, CONTACT_STOP_CAPACITY};
pub use offscreen::{
    AdapterPolicy, AdapterResult, ClippedMeshRequest, ContactPaintSource, CutMeshRequest,
    Offscreen, PreparedScene, PreparedSceneClipRequest, PreparedSceneSource, PreparedSceneTopology,
    PreparedSceneUpdate, PreparedViewportClipRequest, PreparedViewportRequest, RenderDeadline,
    SceneDrawEntry, SculptBufferUpdateStats, SculptFaceUpdate, SculptSurfaceFeedbackRequest,
    SculptTopologyDelta, SculptVertexUpdate, ThumbnailSpec, ViewportSpec,
};
pub use pipeline::live_depth_format;
pub use pipeline::Renderer;
pub use sculpt_cursor::{
    sculpt_surface_light_intensity, sculpt_tool_length, SculptBrushUniform, SculptFeedbackStyle,
    SculptToolShape, SculptToolUniform,
};
pub use texture::GpuTexture;

#[cfg(test)]
#[path = "sculpt_cursor_tests.rs"]
mod sculpt_cursor_tests;
