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
//! - **Right-handed Y-up** coordinate frame.
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
pub mod mesh;
pub mod scene;
pub mod units;

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

pub use bbox::Aabb;
pub use camera::{
    orbit_delta_from_pointer_motion, zoom_factor_from_scroll, Camera, CameraAxisView, CameraPreset,
    CameraProjection, BBOX_FRAME_FILL, CAD_ORBIT_DRAG_GAIN, CAD_ZOOM_SCROLL_SENSITIVITY,
    MIN_ORTHOGRAPHIC_HEIGHT_MM,
};
pub use error::CoreError;
pub use mesh::{
    accumulate_smooth_normals, LiveRayPick, Mesh, MeshBuilder, MeshKind, MeshTexture,
    PrincipalFrame, Vertex,
};
pub use scene::{
    OverlayKind, Scene, SceneMesh, SceneMeshId, ScenePickHit, DEFAULT_UNTEXTURED_MESH_TINT,
};
pub use units::{Millimeters, SourceUnit, UnitConfidence, UnitInterpretation};
