use super::{ThumbnailError, ThumbnailRendererPool};
use glam::{Mat4, Vec2, Vec3};
use occluview_core::{Aabb, Camera, CameraPreset, Mesh, DEFAULT_UNTEXTURED_MESH_TINT};
use occluview_render::{
    GpuCamera, GpuMeshUniform, GpuTexture, Offscreen, SceneDrawEntry, ThumbnailSpec,
};

const THUMBNAIL_PROJECTED_BBOX_FILL: f32 = 0.86;
const THUMBNAIL_PROJECTED_MESH_FILL: f32 = 0.90;
const MIN_THUMBNAIL_ORTHOGRAPHIC_HEIGHT_MM: f32 = 0.01;
const THUMBNAIL_FRAME_SAMPLE_LIMIT: usize = 12_000;
const THUMBNAIL_AREA_SAMPLE_LIMIT: usize = 12_000;
const MAX_SUPERSAMPLED_THUMBNAIL_SIZE_PX: u16 = 256;
const EDGE_ON_FALLBACK_AREA_GAIN: f32 = 2.5;

pub(super) fn render_mesh_thumbnail(
    mesh: Mesh,
    spec: ThumbnailSpec,
) -> Result<Vec<u8>, ThumbnailError> {
    #[cfg(test)]
    let _guard = crate::acquire_render_test_guard();

    let pool = ThumbnailRendererPool::shared();
    match pool
        .with_renderer(|offscreen| render_mesh_thumbnail_with_offscreen(&mesh, spec, offscreen))
    {
        Ok(pixels) => Ok(pixels),
        Err(error) => {
            tracing::warn!(
                ?error,
                "thumbnail renderer failed; retrying once with a fresh device"
            );
            pool.with_renderer(|offscreen| {
                render_mesh_thumbnail_with_offscreen(&mesh, spec, offscreen)
            })
        }
    }
}

pub(super) fn render_mesh_thumbnail_with_offscreen(
    mesh: &Mesh,
    spec: ThumbnailSpec,
    offscreen: &Offscreen,
) -> Result<Vec<u8>, ThumbnailError> {
    let cam = thumbnail_camera_for_mesh(mesh);
    let view = occluview_render::camera_view_matrix(&cam);
    let proj = thumbnail_projection_matrix(&cam);
    let gpu_cam = GpuCamera::new(view, proj, thumbnail_light_dir(&cam), cam.eye());

    let texture = mesh.texture().map(|texture| {
        GpuTexture::upload(
            offscreen.renderer(),
            offscreen.renderer().device(),
            offscreen.renderer().queue(),
            texture,
        )
    });
    let uniform = thumbnail_mesh_uniform(mesh);
    let entries = [SceneDrawEntry {
        mesh,
        uniform: &uniform,
        texture: texture.as_ref(),
    }];
    let render_spec = supersampled_thumbnail_spec(spec);
    let pixels = pollster::block_on(offscreen.render_scene(&entries, &gpu_cam, render_spec))?;
    let pixels = if render_spec.size_px == spec.size_px {
        pixels
    } else {
        downsample_rgba_premultiplied(&pixels, render_spec.size_px, spec.size_px)
    };
    Ok(boost_sparse_thumbnail_visibility(pixels, spec.size_px))
}

pub(super) fn thumbnail_camera_for_mesh(mesh: &Mesh) -> Camera {
    let bbox = mesh.bbox_cached();
    let mut camera = thumbnail_camera_for_bbox(bbox);
    let Some(frame) = projected_mesh_frame(mesh, &camera) else {
        return camera;
    };

    if let Some((fallback_camera, fallback_frame)) =
        edge_on_thumbnail_fallback(mesh, bbox, &camera, frame)
    {
        camera = fallback_camera;
        return fit_thumbnail_camera_to_projected_frame(camera, fallback_frame, bbox);
    }

    fit_thumbnail_camera_to_projected_frame(camera, frame, bbox)
}

fn supersampled_thumbnail_spec(spec: ThumbnailSpec) -> ThumbnailSpec {
    let output_size = spec.size_px.max(1);
    let size_px = if output_size <= MAX_SUPERSAMPLED_THUMBNAIL_SIZE_PX {
        output_size.saturating_mul(2)
    } else {
        output_size
    };
    ThumbnailSpec { size_px, ..spec }
}

pub(super) fn downsample_rgba_premultiplied(
    pixels: &[u8],
    source_size: u16,
    target_size: u16,
) -> Vec<u8> {
    let source_size = usize::from(source_size);
    let target_size = usize::from(target_size.max(1));
    if source_size == target_size {
        return pixels.to_vec();
    }
    let scale = source_size / target_size;
    if scale == 0 || source_size % target_size != 0 {
        return pixels.to_vec();
    }

    let mut out = vec![0u8; target_size * target_size * 4];
    let samples = u32::try_from(scale * scale).unwrap_or(u32::MAX);
    for y in 0..target_size {
        for x in 0..target_size {
            let mut alpha_sum = 0u32;
            let mut red_sum = 0u32;
            let mut green_sum = 0u32;
            let mut blue_sum = 0u32;
            for sy in 0..scale {
                for sx in 0..scale {
                    let src = (((y * scale + sy) * source_size) + (x * scale + sx)) * 4;
                    let alpha = u32::from(pixels[src + 3]);
                    alpha_sum += alpha;
                    red_sum += u32::from(pixels[src]) * alpha;
                    green_sum += u32::from(pixels[src + 1]) * alpha;
                    blue_sum += u32::from(pixels[src + 2]) * alpha;
                }
            }
            let dst = ((y * target_size) + x) * 4;
            let alpha = alpha_sum / samples;
            if alpha_sum > 0 {
                out[dst] = u8::try_from(red_sum / alpha_sum).unwrap_or(u8::MAX);
                out[dst + 1] = u8::try_from(green_sum / alpha_sum).unwrap_or(u8::MAX);
                out[dst + 2] = u8::try_from(blue_sum / alpha_sum).unwrap_or(u8::MAX);
            }
            out[dst + 3] = u8::try_from(alpha).unwrap_or(u8::MAX);
        }
    }
    out
}

fn boost_sparse_thumbnail_visibility(mut pixels: Vec<u8>, size_px: u16) -> Vec<u8> {
    let size = usize::from(size_px.max(1));
    if size > usize::from(MAX_SUPERSAMPLED_THUMBNAIL_SIZE_PX) {
        return pixels;
    }

    let pixel_count = size * size;
    let visible = pixels.chunks_exact(4).filter(|px| px[3] > 0).count();
    if visible == 0 || visible >= (pixel_count / 48).max(8) {
        return pixels;
    }

    let source = pixels.clone();
    for y in 0..size {
        for x in 0..size {
            let src = ((y * size) + x) * 4;
            if source[src + 3] == 0 {
                continue;
            }
            for dy in -1isize..=1 {
                for dx in -1isize..=1 {
                    let Some(nx) = x.checked_add_signed(dx) else {
                        continue;
                    };
                    let Some(ny) = y.checked_add_signed(dy) else {
                        continue;
                    };
                    if nx >= size || ny >= size {
                        continue;
                    }
                    let dst = ((ny * size) + nx) * 4;
                    pixels[dst] = source[src];
                    pixels[dst + 1] = source[src + 1];
                    pixels[dst + 2] = source[src + 2];
                    pixels[dst + 3] = u8::MAX;
                }
            }
        }
    }
    pixels
}

pub(super) fn thumbnail_mesh_uniform(mesh: &Mesh) -> GpuMeshUniform {
    let has_texture = mesh.texture().is_some();
    GpuMeshUniform {
        tint: if has_texture {
            [1.0, 1.0, 1.0, 1.0]
        } else {
            DEFAULT_UNTEXTURED_MESH_TINT
        },
        has_texture: u32::from(has_texture),
        ..GpuMeshUniform::identity()
    }
}

pub(super) fn thumbnail_camera_for_bbox(bbox: Aabb) -> Camera {
    thumbnail_camera_for_preset_bbox(bbox, CameraPreset::Occlusal)
}

fn thumbnail_camera_for_preset_bbox(bbox: Aabb, preset: CameraPreset) -> Camera {
    let mut camera = preset.frame_bbox(bbox, 45.0_f32.to_radians());
    if bbox.is_empty() {
        return camera;
    }

    let projected_span = projected_bbox_frame_span(bbox, &camera);
    if projected_span.is_finite() && projected_span > 0.0 {
        camera.orthographic_height = (projected_span / THUMBNAIL_PROJECTED_BBOX_FILL)
            .max(MIN_THUMBNAIL_ORTHOGRAPHIC_HEIGHT_MM);
    }
    camera
}

fn edge_on_thumbnail_fallback(
    mesh: &Mesh,
    bbox: Aabb,
    current_camera: &Camera,
    current_frame: ProjectedMeshFrame,
) -> Option<(Camera, ProjectedMeshFrame)> {
    let current_score = projected_triangle_coverage_score(mesh, current_camera, current_frame);
    if current_score <= f32::EPSILON {
        return None;
    }

    let mut best = (*current_camera, current_frame, current_score);
    for preset in [
        CameraPreset::Buccal,
        CameraPreset::Lingual,
        CameraPreset::Mesial,
        CameraPreset::Distal,
    ] {
        let camera = thumbnail_camera_for_preset_bbox(bbox, preset);
        let Some(frame) = projected_mesh_frame(mesh, &camera) else {
            continue;
        };
        let score = projected_triangle_coverage_score(mesh, &camera, frame);
        if score > best.2 {
            best = (camera, frame, score);
        }
    }

    (best.2 > current_score * EDGE_ON_FALLBACK_AREA_GAIN).then_some((best.0, best.1))
}

fn fit_thumbnail_camera_to_projected_frame(
    mut camera: Camera,
    frame: ProjectedMeshFrame,
    bbox: Aabb,
) -> Camera {
    if frame.span.is_finite() && frame.span > 0.0 {
        let forward = camera.view_direction();
        let up = camera.view_up();
        let right = forward.cross(up).normalize_or_zero();
        if right.length_squared() > f32::EPSILON && up.length_squared() > f32::EPSILON {
            camera.target += right * frame.center_u + up * frame.center_v;
            camera.orthographic_height = (frame.span / THUMBNAIL_PROJECTED_MESH_FILL)
                .max(MIN_THUMBNAIL_ORTHOGRAPHIC_HEIGHT_MM);
            camera.fit_clip_planes_to_bbox(bbox);
        }
    }
    camera
}

pub(super) fn thumbnail_projection_matrix(camera: &Camera) -> Mat4 {
    // Thumbnails are square: aspect 1.0 through the shared projection.
    occluview_render::camera_ortho_proj_matrix(camera, 1.0)
}

fn thumbnail_light_dir(camera: &Camera) -> Vec3 {
    let forward = camera.view_direction();
    let up = camera.view_up();
    let right = forward.cross(up).normalize_or_zero();
    (-forward + up * 0.32 + right * 0.22).normalize_or_zero()
}

fn projected_bbox_frame_span(bbox: Aabb, camera: &Camera) -> f32 {
    if bbox.is_empty() {
        return 0.0;
    }

    let forward = camera.view_direction();
    let up = camera.view_up();
    let right = forward.cross(up).normalize_or_zero();
    if right.length_squared() <= f32::EPSILON || up.length_squared() <= f32::EPSILON {
        return bbox.half_diagonal() * 2.0;
    }

    let center = bbox.center();
    let mut min_u = f32::INFINITY;
    let mut max_u = f32::NEG_INFINITY;
    let mut min_v = f32::INFINITY;
    let mut max_v = f32::NEG_INFINITY;
    for corner in bbox_corners(bbox) {
        let rel = corner - center;
        let u = rel.dot(right);
        let v = rel.dot(up);
        min_u = min_u.min(u);
        max_u = max_u.max(u);
        min_v = min_v.min(v);
        max_v = max_v.max(v);
    }
    (max_u - min_u).max(max_v - min_v)
}

#[derive(Clone, Copy, Debug)]
struct ProjectedMeshFrame {
    center_u: f32,
    center_v: f32,
    span: f32,
}

fn projected_mesh_frame(mesh: &Mesh, camera: &Camera) -> Option<ProjectedMeshFrame> {
    let vertices = mesh.vertices();
    if vertices.is_empty() {
        return None;
    }

    let forward = camera.view_direction();
    let up = camera.view_up();
    let right = forward.cross(up).normalize_or_zero();
    if right.length_squared() <= f32::EPSILON || up.length_squared() <= f32::EPSILON {
        return None;
    }

    let center = camera.target;
    let stride = (vertices.len() / THUMBNAIL_FRAME_SAMPLE_LIMIT).max(1);
    let mut us = Vec::with_capacity((vertices.len() / stride).max(1));
    let mut vs = Vec::with_capacity(us.capacity());
    for vertex in vertices.iter().step_by(stride) {
        let rel = Vec3::from_array(vertex.position) - center;
        us.push(rel.dot(right));
        vs.push(rel.dot(up));
    }
    if us.is_empty() || vs.is_empty() {
        return None;
    }

    us.sort_by(f32::total_cmp);
    vs.sort_by(f32::total_cmp);
    let (min_u, max_u) = robust_range(&us);
    let (min_v, max_v) = robust_range(&vs);
    let span_u = max_u - min_u;
    let span_v = max_v - min_v;
    let span = span_u.max(span_v);
    if !span.is_finite() || span <= f32::EPSILON {
        return None;
    }

    Some(ProjectedMeshFrame {
        center_u: (min_u + max_u) * 0.5,
        center_v: (min_v + max_v) * 0.5,
        span,
    })
}

fn projected_triangle_coverage_score(
    mesh: &Mesh,
    camera: &Camera,
    frame: ProjectedMeshFrame,
) -> f32 {
    let frame_area = frame.span * frame.span;
    if !frame_area.is_finite() || frame_area <= f32::EPSILON {
        return 0.0;
    }
    projected_triangle_area_score(mesh, camera) / frame_area
}

fn projected_triangle_area_score(mesh: &Mesh, camera: &Camera) -> f32 {
    let indices = mesh.indices();
    if indices.is_empty() {
        return 0.0;
    }
    let vertices = mesh.vertices();
    let forward = camera.view_direction();
    let up = camera.view_up();
    let right = forward.cross(up).normalize_or_zero();
    if right.length_squared() <= f32::EPSILON || up.length_squared() <= f32::EPSILON {
        return 0.0;
    }

    let center = camera.target;
    let triangle_count = indices.len() / 3;
    let stride = (triangle_count / THUMBNAIL_AREA_SAMPLE_LIMIT).max(1);
    let mut area = 0.0_f32;
    for triangle in indices.chunks_exact(3).step_by(stride) {
        let a = projected_vertex_uv(vertices[triangle[0] as usize].position, center, right, up);
        let b = projected_vertex_uv(vertices[triangle[1] as usize].position, center, right, up);
        let c = projected_vertex_uv(vertices[triangle[2] as usize].position, center, right, up);
        area += ((b - a).perp_dot(c - a)).abs() * 0.5;
    }
    area
}

fn projected_vertex_uv(position: [f32; 3], center: Vec3, right: Vec3, up: Vec3) -> Vec2 {
    let rel = Vec3::from_array(position) - center;
    Vec2::new(rel.dot(right), rel.dot(up))
}

fn robust_range(sorted: &[f32]) -> (f32, f32) {
    if sorted.len() < 128 {
        return (sorted[0], sorted[sorted.len() - 1]);
    }
    let trim = (sorted.len() / 100).clamp(1, sorted.len() / 10);
    (sorted[trim], sorted[sorted.len() - 1 - trim])
}

fn bbox_corners(bbox: Aabb) -> [Vec3; 8] {
    let min = bbox.min;
    let max = bbox.max;
    [
        Vec3::new(min.x, min.y, min.z),
        Vec3::new(max.x, min.y, min.z),
        Vec3::new(min.x, max.y, min.z),
        Vec3::new(max.x, max.y, min.z),
        Vec3::new(min.x, min.y, max.z),
        Vec3::new(max.x, min.y, max.z),
        Vec3::new(min.x, max.y, max.z),
        Vec3::new(max.x, max.y, max.z),
    ]
}
