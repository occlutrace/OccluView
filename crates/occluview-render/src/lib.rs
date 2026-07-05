//! `occluview-render` - the wgpu renderer (ADR-0002).
//!
//! Two consumers share this code: the live GUI (`occluview-app`) and the
//! offscreen thumbnail renderer (`occluview-shell`). One pipeline = the
//! Explorer thumbnail is pixel-identical to the in-app frame.
//!
//! ## Layout
//!
//! - [`camera`] - the GPU-side camera uniform (matches the WGSL `Camera`
//!   struct byte-for-byte).
//! - [`gpu`] - GPU mesh upload (vertex/index buffers) from `occluview_core::Mesh`.
//! - [`pipeline`] - render pipeline creation (device + shader + layout).
//! - [`offscreen`] - headless render-to-texture (thumbnails, golden tests).
//!
//! ## Status
//!
//! v1 pipeline: flat/Lambertian shading, vertex colors, depth-tested,
//! indexed draws. WGSL source in `shaders/mesh.wgsl`.

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
pub mod error;
pub mod gpu;
pub mod mesh_uniform;
pub mod offscreen;
pub mod pipeline;
pub mod texture;

pub use camera::GpuCamera;
pub use error::RenderError;
pub use gpu::GpuMesh;
pub use mesh_uniform::GpuMeshUniform;
pub use offscreen::{Offscreen, SceneDrawEntry, ThumbnailSpec};
pub use pipeline::Renderer;
pub use texture::GpuTexture;
