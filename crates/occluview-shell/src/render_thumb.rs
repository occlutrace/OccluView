//! The safe, Windows-agnostic core of thumbnail generation.
//!
//! The COM class (to be implemented in a follow-up PR) calls into
//! [`render_thumbnail`] - this function does all the work and is unit-testable
//! without Windows. It loads the file via `occluview-formats`, frames the
//! camera with the dental occlusal default, and renders an offscreen frame via
//! `occluview-render`.
//!
//! ## Auto-orientation (ADR-0009)
//!
//! Scanners export meshes in arbitrary frames. Before framing the occlusal
//! view we run PCA over the vertices ([`occluview_core::principal_axes`]) and
//! fold the resulting rotation into the camera view matrix. The smallest-
//! variance axis — the thinnest direction of the arch — becomes the occlusal
//! normal pointing at the camera. This makes the thumbnail correct regardless
//! of the source file's orientation, with no per-vertex rewrite.

use crate::thumbnail_format::infer_thumbnail_format;
use crate::ShellError;
use glam::{Mat4, Vec3};
use occluview_core::{principal_axes, Camera, Mesh};
use occluview_formats::dispatch::dispatch_by_kind;
use occluview_render::{GpuCamera, Offscreen, ThumbnailSpec};

/// Load `bytes` (a file with the given lowercase extension, no dot) and render
/// a thumbnail per `spec`. Returns RGBA8 pixels in row-major order, length
/// `spec.size_px * spec.size_px * 4`, top-to-bottom.
///
/// Blocking: runs the offscreen render to completion on the calling thread.
/// The COM stub invokes this on a worker thread (ADR-0005 addendum) under a
/// Job Object with a watchdog.
///
/// # Errors
/// See [`ShellError`]. The COM layer translates any error into a branded
/// placeholder returned to Windows.
pub fn render_thumbnail(
    extension: &str,
    bytes: &[u8],
    spec: ThumbnailSpec,
) -> Result<Vec<u8>, ShellError> {
    render_thumbnail_bytes(Some(extension), bytes, spec)
}

/// Load `bytes` with an optional file extension hint and render a thumbnail.
///
/// This is the entry point for shell streams where Windows may not provide a
/// file path. It never falls back to a fake default extension.
///
/// # Errors
/// Returns [`ShellError::Format`] if inference or parsing fails, and
/// [`ShellError::Render`] if offscreen rendering fails.
pub fn render_thumbnail_bytes(
    extension: Option<&str>,
    bytes: &[u8],
    spec: ThumbnailSpec,
) -> Result<Vec<u8>, ShellError> {
    let kind = infer_thumbnail_format(extension, bytes)?;
    let mesh = dispatch_by_kind(kind, bytes)?;
    render_mesh_thumbnail(mesh, spec)
}

fn render_mesh_thumbnail(mut mesh: Mesh, spec: ThumbnailSpec) -> Result<Vec<u8>, ShellError> {
    let bbox = mesh.bbox();

    // PCA auto-orientation: rotate the *camera frame* so the mesh's thinnest
    // axis aligns with canonical +Y. Cheaper than rewriting every vertex.
    // `orient` maps a world-space point into the canonical dental frame; we
    // compose it before the local look-at so the renderer sees the rotated
    // scene: view = view_local * orient.
    let orient = principal_axes(&sample_vertices(&mesh));

    // Frame against the bbox expressed in the CANONICAL (rotated) frame, so
    // the occlusal framer sees the right vertical depth. Rotate the 8 corners.
    let canonical_bbox = rotate_bbox(bbox, orient);
    let cam = Camera::default().frame_occlusal(canonical_bbox, 45.0_f32.to_radians());
    let view_local = Mat4::look_at_rh(cam.eye(), cam.target, Vec3::Y);
    let view = view_local * Mat4::from_mat3(orient);

    let proj = Mat4::perspective_rh(cam.fovy, 1.0, cam.near, cam.far);
    let gpu_cam = GpuCamera::new(view, proj, Vec3::new(0.4, 0.8, 0.5), cam.eye());

    let offscreen = pollster::block_on(Offscreen::new())?;
    let pixels = pollster::block_on(offscreen.render(&mesh, &gpu_cam, spec))?;
    Ok(pixels)
}

/// The axis-aligned box enclosing `bbox` after rotation by `r`. Used so the
/// occlusal camera framer sees the canonical-frame extents.
fn rotate_bbox(bbox: occluview_core::Aabb, r: glam::Mat3) -> occluview_core::Aabb {
    use occluview_core::Aabb;
    if bbox.is_empty() {
        return Aabb::EMPTY;
    }
    let corners = [
        bbox.min,
        Vec3::new(bbox.min.x, bbox.min.y, bbox.max.z),
        Vec3::new(bbox.min.x, bbox.max.y, bbox.min.z),
        Vec3::new(bbox.min.x, bbox.max.y, bbox.max.z),
        Vec3::new(bbox.max.x, bbox.min.y, bbox.min.z),
        Vec3::new(bbox.max.x, bbox.min.y, bbox.max.z),
        Vec3::new(bbox.max.x, bbox.max.y, bbox.min.z),
        bbox.max,
    ];
    corners
        .into_iter()
        .map(|c| r * c)
        .fold(Aabb::EMPTY, Aabb::enclose_point)
}

/// Sample up to `CAP` vertices (uniform stride) for PCA. PCA is a global
/// second-moment statistic, so a sparse sample is plenty for orientation and
/// keeps the cost bounded for million-vertex scans.
fn sample_vertices(mesh: &Mesh) -> Vec<Vec3> {
    const CAP: usize = 4096;
    let verts = mesh.vertices();
    let n = verts.len();
    if n == 0 {
        return Vec::new();
    }
    if n <= CAP {
        return verts.iter().map(|v| Vec3::from(v.position)).collect();
    }
    let stride = n / CAP;
    (0..n)
        .step_by(stride.max(1))
        .map(|i| Vec3::from(verts[i].position))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_extension_is_a_shell_error() {
        // The render path should surface an error (which the COM layer turns
        // into a placeholder) rather than panic or fake success.
        let res = render_thumbnail("xyz", &[0u8; 4], ThumbnailSpec::default());
        assert!(matches!(res, Err(ShellError::Format(_))));
    }

    #[test]
    fn obj_stream_without_extension_reaches_obj_parser() {
        let res = render_thumbnail_bytes(None, b"v not-a-number\n", ThumbnailSpec::default());
        let Err(ShellError::Format(occluview_formats::FormatError::Malformed { format, .. })) = res
        else {
            panic!("expected OBJ parser error, got {res:?}");
        };
        assert_eq!(format, "OBJ");
    }

    #[test]
    fn threemf_stream_is_rejected_before_rendering() {
        let res = render_thumbnail_bytes(
            Some("3mf"),
            &[0x50, 0x4B, 0x03, 0x04],
            ThumbnailSpec::default(),
        );
        assert!(matches!(
            res,
            Err(ShellError::Format(
                occluview_formats::FormatError::Unsupported { .. }
            ))
        ));
    }

    #[test]
    fn malformed_stl_returns_format_error_without_panic() {
        // Truncated STL header (fewer than 84 bytes) -> FormatError, not panic.
        let res = render_thumbnail("stl", &[0u8; 10], ThumbnailSpec::default());
        assert!(matches!(res, Err(ShellError::Format(_))));
    }
}
