//! Reproducible performance harness (synthetic inputs only, no new deps).
//!
//! Run: `cargo test -p occluview-app --lib perf_harness -- --ignored --nocapture`
//!
//! Cases print wall times, assert functional outcomes, and trip only on
//! order-of-magnitude regressions (generous ceiling, same convention as the
//! structural perf harness). Plain `cargo test` skips them.
//!
//! Fixtures are generated in code (no patient data, no downloads):
//!
//! | area | fixture | status here |
//! |---|---|---|
//! | sculpt small dab | 2-triangle quad, Add brush | executable below |
//! | sculpt large dab | 150x150 grid (~22k verts), Add brush | executable below |
//! | session prepare | same grid through `BrushSession::prepare` | executable below |
//! | repair | duplicate-face tetrahedron | executable below |
//! | alignment | representative scan pair + index build | inventory: no
//! redistributable fixtures in-repo; run against local scans when available |
//! | startup/load | window + GPU + files | inventory: needs a desktop GPU |
//! | first-frame prepare | GPU device + pipelines | inventory: needs a GPU |
//! | orbit/redraw | live viewport frames | inventory: needs a GPU |
//!
//! No timings are recorded here as claims; measure on the target machine.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::expect_used,
    clippy::print_stdout
)]

use crate::sculpt_tool::{mean_uniform_scale, SculptSession};
use glam::Affine3A;
use occluview_core::{
    mesh_edit_buffers_from_mesh, BrushMode, BrushSession, BrushStroke, Mesh, SceneMesh, Vertex,
};
use occluview_render::PreparedSceneTopology;
use std::sync::Arc;
use std::sync::RwLock;
use std::time::Instant;

/// Reject order-of-magnitude regressions while leaving room for CI variance.
fn assert_perf_ceiling(elapsed: std::time::Duration, case: &str) {
    let ceiling = std::time::Duration::from_secs(10);
    assert!(
        elapsed < ceiling,
        "{case} took {elapsed:?}, past the {ceiling:?} regression ceiling"
    );
}

fn quad_mesh() -> Mesh {
    use glam::Vec3;
    Mesh::new(
        Some("perf-quad".to_string()),
        vec![
            Vertex::at(Vec3::new(-1.0, -1.0, 0.0)),
            Vertex::at(Vec3::new(1.0, -1.0, 0.0)),
            Vertex::at(Vec3::new(1.0, 1.0, 0.0)),
            Vertex::at(Vec3::new(-1.0, 1.0, 0.0)),
        ],
        vec![0, 1, 2, 0, 2, 3],
    )
    .expect("perf quad builds")
}

fn grid_mesh(rows: usize, cols: usize, spacing_mm: f32) -> Mesh {
    use glam::Vec3;
    let mut vertices = Vec::with_capacity(rows * cols);
    for j in 0..rows {
        for i in 0..cols {
            vertices.push(Vertex::at(Vec3::new(
                i as f32 * spacing_mm,
                j as f32 * spacing_mm,
                0.0,
            )));
        }
    }
    let mut indices = Vec::with_capacity((rows - 1) * (cols - 1) * 6);
    let idx = |i: usize, j: usize| (j * cols + i) as u32;
    for j in 0..rows - 1 {
        for i in 0..cols - 1 {
            indices.extend_from_slice(&[idx(i, j), idx(i + 1, j), idx(i + 1, j + 1)]);
            indices.extend_from_slice(&[idx(i, j), idx(i + 1, j + 1), idx(i, j + 1)]);
        }
    }
    Mesh::new(Some("perf-grid".to_string()), vertices, indices).expect("perf grid builds")
}

fn session_for(mesh: &Mesh) -> SculptSession {
    let entry = SceneMesh::new(mesh.clone());
    let brush = BrushSession::prepare(&mesh_edit_buffers_from_mesh(mesh)).expect("prepare");
    SculptSession {
        layer_id: entry.id(),
        topology_id: mesh.topology_id(),
        session: brush,
        base_mesh: Arc::new(mesh.clone()),
        shadow: Arc::new(RwLock::new(mesh.vertices().to_vec())),
        topology: PreparedSceneTopology::from_mesh(mesh),
        world_to_local: Affine3A::IDENTITY,
        local_per_world: mean_uniform_scale(&Affine3A::IDENTITY),
        dirty_stroke: false,
        stroke_start_mesh: None,
    }
}

fn dab(center: [f32; 3], radius_mm: f32) -> BrushStroke {
    BrushStroke {
        center,
        radius_mm,
        strength: 1.0,
        view_dir: [0.0, 0.0, -1.0],
    }
}

#[test]
#[ignore = "perf harness: run with --ignored --nocapture"]
fn perf_sculpt_small_dab() {
    let mesh = quad_mesh();
    let mut session = session_for(&mesh);
    let stroke = dab([0.0, 0.0, 0.0], 2.0);
    let first = session.apply_dab(stroke, BrushMode::Add);
    assert!(
        !first.touched.is_empty(),
        "the small dab must touch vertices"
    );
    let start = Instant::now();
    let iterations = 50;
    for _ in 0..iterations {
        session.apply_dab(stroke, BrushMode::Add);
    }
    let elapsed = start.elapsed();
    println!(
        "perf sculpt-small-dab: {iterations} dabs on {} verts took {elapsed:?}",
        mesh.vertices().len()
    );
    assert_perf_ceiling(elapsed, "perf_sculpt_small_dab");
}

#[test]
#[ignore = "perf harness: run with --ignored --nocapture"]
fn perf_sculpt_large_dab() {
    let mesh = grid_mesh(150, 150, 1.0);
    let mut session = session_for(&mesh);
    let stroke = dab([75.0, 75.0, 0.0], 10.0);
    let first = session.apply_dab(stroke, BrushMode::Add);
    assert!(
        !first.touched.is_empty(),
        "the large dab must touch vertices"
    );
    let start = Instant::now();
    let iterations = 5;
    for _ in 0..iterations {
        session.apply_dab(stroke, BrushMode::Add);
    }
    let elapsed = start.elapsed();
    println!(
        "perf sculpt-large-dab: {iterations} dabs on {} verts took {elapsed:?}",
        mesh.vertices().len()
    );
    assert_perf_ceiling(elapsed, "perf_sculpt_large_dab");
}

#[test]
#[ignore = "perf harness: run with --ignored --nocapture"]
fn perf_session_prepare() {
    let mesh = grid_mesh(150, 150, 1.0);
    let buffers = mesh_edit_buffers_from_mesh(&mesh);
    let start = Instant::now();
    let brush = BrushSession::prepare(&buffers).expect("prepare");
    let elapsed = start.elapsed();
    drop(brush);
    println!(
        "perf session-prepare: {} verts took {elapsed:?}",
        mesh.vertices().len()
    );
    assert_perf_ceiling(elapsed, "perf_session_prepare");
}

#[test]
#[ignore = "perf harness: run with --ignored --nocapture"]
fn perf_repair_dirty_tetrahedron() {
    use glam::Vec3;
    use occluview_core::RepairOptions;
    let mesh = Mesh::new(
        Some("perf-dirty-tetra".to_string()),
        vec![
            Vertex::at(Vec3::new(0.0, 0.0, 0.0)),
            Vertex::at(Vec3::new(1.0, 0.0, 0.0)),
            Vertex::at(Vec3::new(0.0, 1.0, 0.0)),
            Vertex::at(Vec3::new(0.0, 0.0, 1.0)),
        ],
        vec![0, 2, 1, 0, 1, 3, 1, 2, 3, 0, 3, 2, 0, 2, 1],
    )
    .expect("dirty tetra builds");
    let start = Instant::now();
    let result = occluview_core::repair_mesh_in_mesh(&mesh, RepairOptions::default());
    let elapsed = start.elapsed();
    assert!(result.is_ok(), "the dirty tetrahedron must repair");
    println!("perf repair-dirty-tetra: took {elapsed:?}");
    assert_perf_ceiling(elapsed, "perf_repair_dirty_tetrahedron");
}
