//! Shared fixtures and setup helpers for workspace tests.

use crate::{CoreError, Mesh, MeshBuilder, Vertex};
use glam::Vec3;
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};

/// Builds the unit right triangle used by mesh and scene tests.
pub fn simple_triangle_mesh(name: Option<&str>) -> Option<Mesh> {
    Mesh::new(
        name.map(str::to_owned),
        vec![
            Vertex::at(Vec3::ZERO),
            Vertex::at(Vec3::X),
            Vertex::at(Vec3::Y),
        ],
        vec![0, 1, 2],
    )
    .ok()
}

/// Builds two separate unit triangles positioned along the X axis.
pub fn two_triangle_mesh(name: Option<&str>) -> Option<Mesh> {
    Mesh::new(
        name.map(str::to_owned),
        vec![
            Vertex::at(Vec3::ZERO),
            Vertex::at(Vec3::X),
            Vertex::at(Vec3::Y),
            Vertex::at(Vec3::new(2.0, 0.0, 0.0)),
            Vertex::at(Vec3::new(3.0, 0.0, 0.0)),
            Vertex::at(Vec3::new(2.0, 1.0, 0.0)),
        ],
        vec![0, 1, 2, 3, 4, 5],
    )
    .ok()
}

/// Builds the square quad used by sculpt worker tests.
pub fn quad_mesh(name: Option<&str>) -> Option<Mesh> {
    Mesh::new(
        name.map(str::to_owned),
        vec![
            Vertex::at(Vec3::new(-1.0, -1.0, 0.0)),
            Vertex::at(Vec3::new(1.0, -1.0, 0.0)),
            Vertex::at(Vec3::new(1.0, 1.0, 0.0)),
            Vertex::at(Vec3::new(-1.0, 1.0, 0.0)),
        ],
        vec![0, 1, 2, 0, 2, 3],
    )
    .ok()
}

/// Builds the colored, normaled, UV-mapped sample triangle used by writer tests.
pub fn colored_uv_triangle_mesh(name: Option<&str>) -> Result<Mesh, CoreError> {
    Mesh::new(
        name.map(str::to_owned),
        vec![
            Vertex::at(Vec3::ZERO)
                .with_normal(Vec3::Z)
                .with_color([210, 180, 120, 255])
                .with_uv([0.0, 0.0]),
            Vertex::at(Vec3::X)
                .with_normal(Vec3::Z)
                .with_color([220, 170, 110, 255])
                .with_uv([1.0, 0.0]),
            Vertex::at(Vec3::Y)
                .with_normal(Vec3::Z)
                .with_color([230, 160, 100, 255])
                .with_uv([0.0, 1.0]),
        ],
        vec![0, 1, 2],
    )
}

/// Builds the folded 5 by 3 ridge surface used by sculpt tests.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap
)]
pub fn coarse_ridge_mesh() -> Result<Mesh, CoreError> {
    let mut vertices = Vec::new();
    for j in 0..3usize {
        for i in 0..5usize {
            let x = i as f32 * 4.0 - 8.0;
            let y = j as f32 * 4.0 - 4.0;
            let z = if j == 1 { 4.0 } else { 0.0 };
            vertices.push(Vertex::at(Vec3::new(x, y, z)));
        }
    }
    let mut indices = Vec::new();
    let idx = |i: usize, j: usize| (j * 5 + i) as u32;
    for j in 0..2usize {
        for i in 0..4usize {
            indices.extend_from_slice(&[idx(i, j), idx(i + 1, j), idx(i + 1, j + 1)]);
            indices.extend_from_slice(&[idx(i, j), idx(i + 1, j + 1), idx(i, j + 1)]);
        }
    }
    Mesh::new(Some("coarse-ridge".to_string()), vertices, indices)
}

/// Builds the centered, +Z-facing triangle shared by renderer tests.
pub fn render_triangle_mesh() -> Result<Mesh, CoreError> {
    let mut builder = MeshBuilder::new();
    let a = builder.push_vertex(Vertex::at(Vec3::new(-0.5, -0.5, 0.0)).with_normal(Vec3::Z));
    let b = builder.push_vertex(Vertex::at(Vec3::new(0.5, -0.5, 0.0)).with_normal(Vec3::Z));
    let c = builder.push_vertex(Vertex::at(Vec3::new(0.0, 0.5, 0.0)).with_normal(Vec3::Z));
    builder.push_triangle(a, b, c);
    builder.build()
}

/// Builds the UV-mapped triangle used by renderer texture tests.
pub fn textured_render_triangle_mesh() -> Result<Mesh, CoreError> {
    let mut builder = MeshBuilder::new();
    let a = builder.push_vertex(
        Vertex::at(Vec3::new(-0.5, -0.5, 0.0))
            .with_normal(Vec3::Z)
            .with_uv([0.0, 0.0]),
    );
    let b = builder.push_vertex(
        Vertex::at(Vec3::new(0.5, -0.5, 0.0))
            .with_normal(Vec3::Z)
            .with_uv([1.0, 0.0]),
    );
    let c = builder.push_vertex(
        Vertex::at(Vec3::new(0.0, 0.5, 0.0))
            .with_normal(Vec3::Z)
            .with_uv([0.5, 1.0]),
    );
    builder.push_triangle(a, b, c);
    builder.build()
}

/// Returns the shared one-triangle GLB fixture's JSON and BIN chunk payloads.
pub fn minimal_triangle_glb_chunks() -> (&'static [u8], Vec<u8>) {
    let json = br#"{"asset":{"version":"2.0"},
"scenes":[{"nodes":[0]}],
"nodes":[{"mesh":0}],
"meshes":[{"primitives":[{"attributes":{"POSITION":0},"indices":1}]}],
"accessors":[{"bufferView":0,"count":3,"type":"VEC3","componentType":5126},
             {"bufferView":1,"count":3,"type":"SCALAR","componentType":5125}],
"bufferViews":[{"buffer":0,"byteLength":36},{"buffer":0,"byteOffset":36,"byteLength":12}],
"buffers":[{"byteLength":48}]}"#;
    let mut bin = Vec::with_capacity(48);
    for value in [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0] {
        bin.extend_from_slice(&value.to_le_bytes());
    }
    for index in 0u32..3 {
        bin.extend_from_slice(&index.to_le_bytes());
    }
    (json, bin)
}

/// Serializes an STL triangle record: normal, three corners, and zero attributes.
pub fn append_binary_stl_triangle(out: &mut Vec<u8>, triangle: &[f32; 12]) {
    for &value in triangle {
        out.extend_from_slice(&value.to_le_bytes());
    }
    out.extend_from_slice(&[0, 0]);
}

/// Serializes an STL facet from a normal and its three corner positions.
pub fn append_binary_stl_facet(
    out: &mut Vec<u8>,
    normal: [f32; 3],
    a: [f32; 3],
    b: [f32; 3],
    c: [f32; 3],
) {
    let triangle = [
        normal[0], normal[1], normal[2], a[0], a[1], a[2], b[0], b[1], b[2], c[0], c[1], c[2],
    ];
    append_binary_stl_triangle(out, &triangle);
}

/// Builds a binary STL with a zero-filled 80-byte header.
pub fn binary_stl(triangles: &[[f32; 12]]) -> Vec<u8> {
    binary_stl_with_header(&[0; 80], triangles)
}

/// Builds a binary STL with the supplied 80-byte header.
#[allow(clippy::cast_possible_truncation)]
pub fn binary_stl_with_header(header: &[u8; 80], triangles: &[[f32; 12]]) -> Vec<u8> {
    let mut out = Vec::with_capacity(84 + triangles.len() * 50);
    out.extend_from_slice(header);
    out.extend_from_slice(&(triangles.len() as u32).to_le_bytes());
    for triangle in triangles {
        append_binary_stl_triangle(&mut out, triangle);
    }
    out
}

/// Acquires the renderer-test lock and prepares a private XDG runtime directory on Unix.
pub fn acquire_render_test_guard(runtime_name: &str) -> MutexGuard<'static, ()> {
    static RENDER_TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    ensure_test_runtime_dir(runtime_name);
    RENDER_TEST_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

#[cfg(unix)]
#[allow(clippy::panic)]
fn ensure_test_runtime_dir(runtime_name: &str) {
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    static RUNTIME_DIR: OnceLock<PathBuf> = OnceLock::new();
    let runtime_dir = RUNTIME_DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("occluview-{runtime_name}-wgpu-runtime"));
        let create = std::fs::create_dir_all(&dir);
        assert!(
            create.is_ok(),
            "create {runtime_name} test runtime dir: {create:?}"
        );

        let permissions = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
        assert!(
            permissions.is_ok(),
            "set {runtime_name} test runtime dir permissions: {permissions:?}"
        );
        dir
    });

    if std::env::var_os("XDG_RUNTIME_DIR").is_none() {
        std::env::set_var("XDG_RUNTIME_DIR", runtime_dir);
    }
}

#[cfg(not(unix))]
fn ensure_test_runtime_dir(_: &str) {}
