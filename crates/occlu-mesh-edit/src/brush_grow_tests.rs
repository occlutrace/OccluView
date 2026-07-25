//! Tests for dynamic-topology densification under the Smooth brush.
//!
//! The fixture is the case the operator named: a coarse crease where two large
//! facets meet at a sharp dihedral, far coarser than the brush. Without
//! densification a held Smooth stroke there is a no-op — there are no vertices
//! on the crease to relax.

use std::collections::HashMap;

use glam::Vec3;

use crate::brush::{BrushMode, BrushSession, BrushStroke};
use crate::{EditVertex, MeshEditBuffers, MeshTopology};

/// Lattice spacing of the coarse fixture, in mm — many times the brush radius
/// used in these tests, so every crease edge starts far too long to smooth.
const TENT_SPACING: f32 = 4.0;
/// Ridge height, giving 45-degree slopes and a 90-degree crease.
const TENT_HEIGHT: f32 = 4.0;
/// Lattice half-extent in cells.
const TENT_CELLS: usize = 6;

/// A coarse tent: a 7x7 lattice at 4mm spacing, flat except for a single sharp
/// ridge running along `y = 0`. The ridge is the coarse edge; its interior
/// vertices are free, its lattice border is an open boundary.
fn coarse_tent() -> MeshEditBuffers {
    let n = TENT_CELLS + 1;
    let half = TENT_SPACING * (TENT_CELLS as f32) / 2.0;
    let mut vertices = Vec::with_capacity(n * n);
    for j in 0..n {
        for i in 0..n {
            let x = i as f32 * TENT_SPACING - half;
            let y = j as f32 * TENT_SPACING - half;
            let z = if (y).abs() < f32::EPSILON {
                TENT_HEIGHT
            } else {
                0.0
            };
            vertices.push(EditVertex::at([x, y, z]));
        }
    }
    let mut indices = Vec::with_capacity(TENT_CELLS * TENT_CELLS * 6);
    let idx = |i: usize, j: usize| (j * n + i) as u32;
    for j in 0..TENT_CELLS {
        for i in 0..TENT_CELLS {
            indices.extend_from_slice(&[idx(i, j), idx(i + 1, j), idx(i + 1, j + 1)]);
            indices.extend_from_slice(&[idx(i, j), idx(i + 1, j + 1), idx(i, j + 1)]);
        }
    }
    let mut mesh = MeshEditBuffers {
        vertices,
        indices,
        topology: MeshTopology::TriangleMesh,
    };
    crate::recompute_all_normals(&mut mesh.vertices, &mesh.indices).expect("seed normals");
    mesh
}

/// Explode an indexed mesh into STL-style soup: every corner gets its own
/// vertex, exactly as an STL reader hands it over.
fn as_soup(mesh: &MeshEditBuffers) -> MeshEditBuffers {
    let mut vertices = Vec::with_capacity(mesh.indices.len());
    let mut indices = Vec::with_capacity(mesh.indices.len());
    for &index in &mesh.indices {
        indices.push(vertices.len() as u32);
        vertices.push(mesh.vertices[index as usize]);
    }
    MeshEditBuffers {
        vertices,
        indices,
        topology: MeshTopology::TriangleMesh,
    }
}

fn stroke_at(center: Vec3, radius_mm: f32, strength: f32) -> BrushStroke {
    BrushStroke {
        center: center.to_array(),
        radius_mm,
        strength,
        view_dir: [0.0, 0.0, -1.0],
    }
}

/// Snapshot the session's live buffers as an editable mesh.
fn live_mesh(session: &BrushSession) -> MeshEditBuffers {
    MeshEditBuffers {
        vertices: session.vertices().to_vec(),
        indices: session.indices().to_vec(),
        topology: MeshTopology::TriangleMesh,
    }
}

fn position(mesh: &MeshEditBuffers, index: u32) -> Vec3 {
    Vec3::from_array(mesh.vertices[index as usize].position)
}

fn vertices_within(mesh: &MeshEditBuffers, center: Vec3, radius: f32) -> usize {
    mesh.vertices
        .iter()
        .filter(|vertex| Vec3::from_array(vertex.position).distance(center) <= radius)
        .count()
}

fn triangles_reaching(mesh: &MeshEditBuffers, center: Vec3, radius: f32) -> usize {
    mesh.indices
        .chunks_exact(3)
        .filter(|triangle| {
            triangle
                .iter()
                .any(|&corner| position(mesh, corner).distance(center) <= radius)
        })
        .count()
}

fn face_normal(mesh: &MeshEditBuffers, triangle: &[u32]) -> Vec3 {
    let a = position(mesh, triangle[0]);
    let b = position(mesh, triangle[1]);
    let c = position(mesh, triangle[2]);
    (b - a).cross(c - a)
}

/// Dihedral angles (degrees, 0 = coplanar) of every interior edge whose
/// midpoint falls inside the measurement sphere, as `(max, standard deviation,
/// count)`. This is the "is the crease still sharp" measurement.
fn dihedral_stats(mesh: &MeshEditBuffers, center: Vec3, radius: f32) -> (f32, f32, usize) {
    let mut edges: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
    for (triangle_index, triangle) in mesh.indices.chunks_exact(3).enumerate() {
        for (a, b) in [
            (triangle[0], triangle[1]),
            (triangle[1], triangle[2]),
            (triangle[2], triangle[0]),
        ] {
            let key = if a <= b { (a, b) } else { (b, a) };
            edges.entry(key).or_default().push(triangle_index);
        }
    }
    let mut angles: Vec<f32> = Vec::new();
    for ((a, b), faces) in &edges {
        if faces.len() != 2 {
            continue;
        }
        let midpoint = (position(mesh, *a) + position(mesh, *b)) * 0.5;
        if midpoint.distance(center) > radius {
            continue;
        }
        let left = face_normal(mesh, &mesh.indices[faces[0] * 3..faces[0] * 3 + 3]);
        let right = face_normal(mesh, &mesh.indices[faces[1] * 3..faces[1] * 3 + 3]);
        if left.length_squared() <= f32::EPSILON || right.length_squared() <= f32::EPSILON {
            continue;
        }
        let cosine = left.normalize().dot(right.normalize()).clamp(-1.0, 1.0);
        angles.push(cosine.acos().to_degrees());
    }
    if angles.is_empty() {
        return (0.0, 0.0, 0);
    }
    let max = angles.iter().copied().fold(0.0_f32, f32::max);
    let mean = angles.iter().sum::<f32>() / angles.len() as f32;
    let variance = angles.iter().map(|a| (a - mean).powi(2)).sum::<f32>() / angles.len() as f32;
    (max, variance.sqrt(), angles.len())
}

fn surface_area(mesh: &MeshEditBuffers) -> f64 {
    mesh.indices
        .chunks_exact(3)
        .map(|triangle| f64::from(face_normal(mesh, triangle).length()) * 0.5)
        .sum()
}

/// Distance from `point` to triangle `a b c`, written independently of the
/// kernel's own routine: project onto the plane, keep the projection when its
/// barycentric coordinates are all non-negative, otherwise clamp to the nearest
/// of the three edge segments.
fn point_triangle_distance(point: Vec3, a: Vec3, b: Vec3, c: Vec3) -> f32 {
    let normal = (b - a).cross(c - a);
    let area_twice = normal.length();
    if area_twice > 1e-12 {
        let unit = normal / area_twice;
        let projected = point - unit * (point - a).dot(unit);
        let alpha = (b - projected).cross(c - projected).dot(unit) / area_twice;
        let beta = (c - projected).cross(a - projected).dot(unit) / area_twice;
        let gamma = 1.0 - alpha - beta;
        if alpha >= 0.0 && beta >= 0.0 && gamma >= 0.0 {
            return projected.distance(point);
        }
    }
    [(a, b), (b, c), (c, a)]
        .into_iter()
        .map(|(start, end)| {
            let span = end - start;
            let length_squared = span.length_squared();
            let t = if length_squared > 1e-20 {
                ((point - start).dot(span) / length_squared).clamp(0.0, 1.0)
            } else {
                0.0
            };
            (start + span * t).distance(point)
        })
        .fold(f32::MAX, f32::min)
}

fn distance_to_surface(point: Vec3, mesh: &MeshEditBuffers) -> f32 {
    mesh.indices
        .chunks_exact(3)
        .map(|triangle| {
            point_triangle_distance(
                point,
                position(mesh, triangle[0]),
                position(mesh, triangle[1]),
                position(mesh, triangle[2]),
            )
        })
        .fold(f32::MAX, f32::min)
}

/// Deterministic barycentric samples across every triangle of `mesh`.
fn sample_surface(mesh: &MeshEditBuffers) -> Vec<Vec3> {
    const STEPS: usize = 4;
    let mut samples = Vec::new();
    for triangle in mesh.indices.chunks_exact(3) {
        let a = position(mesh, triangle[0]);
        let b = position(mesh, triangle[1]);
        let c = position(mesh, triangle[2]);
        for i in 0..=STEPS {
            for j in 0..=(STEPS - i) {
                let alpha = i as f32 / STEPS as f32;
                let beta = j as f32 / STEPS as f32;
                samples.push(a * alpha + b * beta + c * (1.0 - alpha - beta));
            }
        }
    }
    samples
}

/// Raw bits of every position and normal, for a bit-for-bit comparison.
fn session_bits(session: &BrushSession) -> Vec<u32> {
    let mut bits: Vec<u32> = Vec::with_capacity(session.vertex_count() * 6);
    for vertex in session.vertices() {
        for component in vertex.position.iter().chain(vertex.normal.iter()) {
            bits.push(component.to_bits());
        }
    }
    bits.extend(session.indices().iter().copied());
    bits
}

/// Interior edges used by exactly two triangles, and edges used by any other
/// count — a split that only rewired one side of an edge would show up here as
/// a T-junction.
fn edge_use_histogram(mesh: &MeshEditBuffers) -> HashMap<usize, usize> {
    let mut uses: HashMap<(u32, u32), usize> = HashMap::new();
    for triangle in mesh.indices.chunks_exact(3) {
        for (a, b) in [
            (triangle[0], triangle[1]),
            (triangle[1], triangle[2]),
            (triangle[2], triangle[0]),
        ] {
            let key = if a <= b { (a, b) } else { (b, a) };
            *uses.entry(key).or_insert(0) += 1;
        }
    }
    let mut histogram: HashMap<usize, usize> = HashMap::new();
    for count in uses.values() {
        *histogram.entry(*count).or_insert(0) += 1;
    }
    histogram
}

/// Open-boundary edges of `mesh`, as endpoint index pairs.
fn border_edges(mesh: &MeshEditBuffers) -> Vec<(u32, u32)> {
    let mut uses: HashMap<(u32, u32), usize> = HashMap::new();
    for triangle in mesh.indices.chunks_exact(3) {
        for (a, b) in [
            (triangle[0], triangle[1]),
            (triangle[1], triangle[2]),
            (triangle[2], triangle[0]),
        ] {
            let key = if a <= b { (a, b) } else { (b, a) };
            *uses.entry(key).or_insert(0) += 1;
        }
    }
    let mut edges: Vec<(u32, u32)> = uses
        .into_iter()
        .filter_map(|(key, count)| (count == 1).then_some(key))
        .collect();
    edges.sort_unstable();
    edges
}

/// Distance from `point` to the ORIGINAL mesh's open-boundary polyline. A
/// pinned rim means every boundary vertex — original or minted by a split —
/// still sits exactly on it.
fn distance_to_border(point: Vec3, mesh: &MeshEditBuffers, edges: &[(u32, u32)]) -> f32 {
    edges
        .iter()
        .map(|&(a, b)| {
            let start = position(mesh, a);
            let span = position(mesh, b) - start;
            let length_squared = span.length_squared();
            let t = if length_squared > 1e-20 {
                ((point - start).dot(span) / length_squared).clamp(0.0, 1.0)
            } else {
                0.0
            };
            (start + span * t).distance(point)
        })
        .fold(f32::MAX, f32::min)
}

const CREASE_CENTER: Vec3 = Vec3::new(0.0, 0.0, TENT_HEIGHT);
const BRUSH_RADIUS: f32 = 3.5;

#[test]
fn smooth_densifies_a_coarse_crease_and_takes_the_dihedral_down() {
    let mesh = coarse_tent();
    let before_vertices = vertices_within(&mesh, CREASE_CENTER, BRUSH_RADIUS);
    let before_triangles = triangles_reaching(&mesh, CREASE_CENTER, BRUSH_RADIUS);
    let (before_max, before_spread, before_edges) =
        dihedral_stats(&mesh, CREASE_CENTER, BRUSH_RADIUS);
    assert!(
        before_max > 80.0,
        "fixture must start as a sharp crease, got {before_max} degrees over {before_edges} edges"
    );

    let mut session = BrushSession::prepare(&mesh).expect("prepare");
    let stroke = stroke_at(CREASE_CENTER, BRUSH_RADIUS, 1.0);
    let mut added = 0usize;
    for _ in 0..24 {
        added += session
            .apply_stroke(stroke, BrushMode::Smooth)
            .added_vertices;
    }
    let after = live_mesh(&session);
    let after_vertices = vertices_within(&after, CREASE_CENTER, BRUSH_RADIUS);
    let after_triangles = triangles_reaching(&after, CREASE_CENTER, BRUSH_RADIUS);
    let (after_max, after_spread, after_edges) =
        dihedral_stats(&after, CREASE_CENTER, BRUSH_RADIUS);

    // (a) many more triangles in the brushed region.
    assert!(added > 0, "the stroke must have added vertices");
    assert!(
        after_triangles >= before_triangles * 8,
        "brushed region must densify: {before_triangles} -> {after_triangles} triangles"
    );
    assert!(
        after_vertices >= before_vertices * 8,
        "brushed region must densify: {before_vertices} -> {after_vertices} vertices"
    );

    // (b) a measurably lower dihedral spread. Across the whole disc the spread
    // must collapse; the MAXIMUM only falls right down in the core, because the
    // rim is where the flattened patch meets the crease the brush never reached
    // — a transition that has to carry the leftover angle somewhere.
    assert!(
        after_edges > before_edges * 10,
        "the measurement must see far more edges after refinement \
         ({before_edges} -> {after_edges})"
    );
    assert!(
        after_spread < before_spread / 3.0,
        "dihedral spread must collapse: {before_spread} -> {after_spread} degrees \
         (max {before_max} -> {after_max})"
    );
    let core = BRUSH_RADIUS * 0.6;
    let (before_core, _, before_core_edges) = dihedral_stats(&mesh, CREASE_CENTER, core);
    let (after_core, _, after_core_edges) = dihedral_stats(&after, CREASE_CENTER, core);
    assert!(
        before_core > 80.0 && before_core_edges > 0,
        "the core must start sharp: {before_core} degrees over {before_core_edges} edges"
    );
    assert!(
        after_core < before_core / 5.0,
        "the brushed core must flatten: max dihedral {before_core} -> {after_core} degrees \
         over {after_core_edges} edges"
    );

    // (c) vertices only where the brush passed. Smoothing may nudge a new
    // vertex a little after it is born, so allow a small margin over the disc.
    for vertex in &after.vertices[mesh.vertices.len()..] {
        let distance = Vec3::from_array(vertex.position).distance(CREASE_CENTER);
        assert!(
            distance <= BRUSH_RADIUS * 1.25,
            "a vertex appeared {distance}mm out, beyond the {BRUSH_RADIUS}mm brush"
        );
    }
    // ... and nothing was added away from the crease: the far corner of the
    // lattice must be untouched, positions included.
    assert_eq!(after.vertices[0].position, mesh.vertices[0].position);
}

#[test]
fn densification_leaves_the_surface_exactly_where_it_was() {
    /// Stated tolerance for "densification does not move the surface". A
    /// midpoint split is exact in exact arithmetic, so the whole budget is
    /// float rounding — a tenth of a micron is orders of magnitude above it and
    /// orders of magnitude below anything dental.
    const TOLERANCE_MM: f32 = 1e-4;

    let mesh = coarse_tent();
    let mut session = BrushSession::prepare(&mesh).expect("prepare");
    // Refinement only: no displacement, so any deviation is the split's fault.
    let mut added = 0usize;
    for step in 0..6 {
        let center = Vec3::new(step as f32 * 1.5 - 4.0, 0.0, TENT_HEIGHT);
        added += session.refine_dab(center, BRUSH_RADIUS);
    }
    assert!(added > 100, "the sweep must densify, added {added}");
    let refined = live_mesh(&session);

    // The refined mesh covers exactly the same surface: no vertex drifted off
    // it, and no part of the original is left uncovered.
    let mut worst_out = 0.0_f32;
    for point in sample_surface(&refined) {
        worst_out = worst_out.max(distance_to_surface(point, &mesh));
    }
    let mut worst_in = 0.0_f32;
    for point in sample_surface(&mesh) {
        worst_in = worst_in.max(distance_to_surface(point, &refined));
    }
    assert!(
        worst_out <= TOLERANCE_MM,
        "refined surface drifted {worst_out}mm off the original"
    );
    assert!(
        worst_in <= TOLERANCE_MM,
        "original surface is {worst_in}mm away from the refined one"
    );

    // Area is the independent check that no facet was lost or double-covered.
    let before_area = surface_area(&mesh);
    let after_area = surface_area(&refined);
    assert!(
        (after_area - before_area).abs() <= before_area * 1e-4,
        "surface area changed: {before_area} -> {after_area}"
    );
}

#[test]
fn a_refined_surface_stays_manifold_with_no_t_junctions() {
    let mesh = coarse_tent();
    let before = edge_use_histogram(&mesh);
    let mut session = BrushSession::prepare(&mesh).expect("prepare");
    for _ in 0..12 {
        session.apply_stroke(
            stroke_at(CREASE_CENTER, BRUSH_RADIUS, 1.0),
            BrushMode::Smooth,
        );
    }
    let after = edge_use_histogram(&live_mesh(&session));
    // A closed interior edge is used twice, a border edge once. Densification
    // must not invent an edge used three times or leave a half-split edge.
    let mut before_kinds: Vec<usize> = before.keys().copied().collect();
    let mut after_kinds: Vec<usize> = after.keys().copied().collect();
    before_kinds.sort_unstable();
    after_kinds.sort_unstable();
    assert_eq!(
        before_kinds, after_kinds,
        "edge-use kinds changed: {before:?} -> {after:?}"
    );
    assert!(
        !after.contains_key(&3) && after.keys().all(|kind| *kind <= 2),
        "no edge may end up shared by more than two triangles: {after:?}"
    );
    assert!(
        after.get(&1).copied().unwrap_or_default() >= before.get(&1).copied().unwrap_or_default(),
        "border edges may be halved but never orphaned: {before:?} -> {after:?}"
    );
    assert!(
        after.get(&2).copied().unwrap_or_default() > before.get(&2).copied().unwrap_or_default(),
        "the interior must have gained edges: {before:?} -> {after:?}"
    );
}

#[test]
fn a_densified_stroke_is_bit_identical_across_thread_counts() {
    let mesh = coarse_tent();
    let run = |threads: usize| -> Vec<u32> {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .expect("thread pool");
        pool.install(|| {
            let mut session = BrushSession::prepare(&mesh).expect("prepare");
            for step in 0..8 {
                let center = Vec3::new(step as f32 * 0.8 - 3.0, 0.0, TENT_HEIGHT);
                session.apply_stroke(stroke_at(center, BRUSH_RADIUS, 0.8), BrushMode::Smooth);
            }
            session_bits(&session)
        })
    };
    let single = run(1);
    let many = run(4);
    assert!(
        single.len() > 6 * mesh.vertices.len(),
        "the stroke must grow the mesh"
    );
    assert_eq!(
        single, many,
        "densified smoothing must be bit-identical regardless of thread count"
    );
}

#[test]
fn session_growth_stays_inside_the_stated_bound_under_a_pathological_stroke() {
    let mesh = coarse_tent();
    let mut session = BrushSession::prepare(&mesh).expect("prepare");
    session.set_added_vertex_budget(600);
    let ceiling = session.prepared_vertex_count() + session.added_vertex_budget();
    // Sweep the whole lattice with a shrinking brush: the detail target shrinks
    // with the radius, so without the bound this never stops splitting.
    for pass in 0..6 {
        let radius = BRUSH_RADIUS / (1.0 + pass as f32);
        for step in 0..24 {
            let center = Vec3::new(step as f32 - 12.0, 0.0, TENT_HEIGHT);
            session.apply_stroke(stroke_at(center, radius, 1.0), BrushMode::Smooth);
            assert!(
                session.vertex_count() <= ceiling,
                "growth bound broken: {} vertices vs ceiling {ceiling}",
                session.vertex_count()
            );
        }
    }
    assert_eq!(
        session.vertex_count(),
        ceiling,
        "a pathological stroke should spend the whole budget and stop exactly there"
    );
}

#[test]
fn a_held_stroke_converges_instead_of_growing_forever() {
    let mesh = coarse_tent();
    let mut session = BrushSession::prepare(&mesh).expect("prepare");
    let stroke = stroke_at(CREASE_CENTER, BRUSH_RADIUS, 1.0);
    let mut per_dab = Vec::new();
    for _ in 0..40 {
        per_dab.push(
            session
                .apply_stroke(stroke, BrushMode::Smooth)
                .added_vertices,
        );
    }
    assert!(per_dab[0] > 0, "the first dab must densify");
    assert!(
        session.vertex_count() < session.prepared_vertex_count() + session.added_vertex_budget(),
        "convergence, not the budget, must be what stops a held stroke"
    );
    // Growth collapses to a trickle: relaxation keeps nudging vertices, so an
    // occasional edge creeps back over target, but the region is a fixed point
    // rather than a treadmill.
    let tail: usize = per_dab[10..].iter().sum();
    assert!(
        tail * 20 <= per_dab[0],
        "a held stroke must reach a fixed point; still adding {tail} vertices after ten dabs \
         (per dab: {per_dab:?})"
    );
}

#[test]
fn densification_never_moves_an_open_boundary() {
    let mesh = coarse_tent();
    let half = TENT_SPACING * (TENT_CELLS as f32) / 2.0;
    let original_border = border_edges(&mesh);
    let mut session = BrushSession::prepare(&mesh).expect("prepare");
    // Brush right along the lattice border, where splitting mints brand-new
    // boundary vertices that must inherit the pin.
    for step in 0..12 {
        let center = Vec3::new(step as f32 - 6.0, -half, 0.0);
        session.apply_stroke(stroke_at(center, BRUSH_RADIUS, 1.0), BrushMode::Smooth);
    }
    // ... and across the rim end of the ridge, where the border is not flat.
    for step in 0..6 {
        let center = Vec3::new(-half, step as f32 - 3.0, 0.0);
        session.apply_stroke(stroke_at(center, BRUSH_RADIUS, 1.0), BrushMode::Smooth);
    }
    let after = live_mesh(&session);
    assert!(
        after.vertices.len() > mesh.vertices.len(),
        "the border stroke must have densified"
    );
    let refined_border = border_edges(&after);
    assert!(
        refined_border.len() > original_border.len(),
        "the rim itself must have subdivided: {} -> {} edges",
        original_border.len(),
        refined_border.len()
    );
    for &(a, b) in &refined_border {
        for corner in [a, b] {
            let point = position(&after, corner);
            let off = distance_to_border(point, &mesh, &original_border);
            assert!(
                off <= 1e-5,
                "a boundary vertex left the original rim by {off}mm at {point:?}"
            );
        }
    }
}

#[test]
fn an_already_fine_mesh_is_not_densified() {
    // A 0.1mm lattice under a 3.5mm brush is already an order of magnitude
    // finer than the detail target, so the stroke must be pure relaxation.
    let n = 41usize;
    let spacing = 0.1_f32;
    let mut vertices = Vec::with_capacity(n * n);
    for j in 0..n {
        for i in 0..n {
            let x = i as f32 * spacing - spacing * (n as f32 - 1.0) / 2.0;
            let y = j as f32 * spacing - spacing * (n as f32 - 1.0) / 2.0;
            let z = ((i * 7 + j * 13) % 5) as f32 * 0.002;
            vertices.push(EditVertex::at([x, y, z]));
        }
    }
    let mut indices = Vec::new();
    let idx = |i: usize, j: usize| (j * n + i) as u32;
    for j in 0..n - 1 {
        for i in 0..n - 1 {
            indices.extend_from_slice(&[idx(i, j), idx(i + 1, j), idx(i + 1, j + 1)]);
            indices.extend_from_slice(&[idx(i, j), idx(i + 1, j + 1), idx(i, j + 1)]);
        }
    }
    let mut mesh = MeshEditBuffers {
        vertices,
        indices,
        topology: MeshTopology::TriangleMesh,
    };
    crate::recompute_all_normals(&mut mesh.vertices, &mesh.indices).expect("seed normals");

    let mut session = BrushSession::prepare(&mesh).expect("prepare");
    let mut outcome_touched = 0usize;
    for _ in 0..8 {
        let outcome =
            session.apply_stroke(stroke_at(Vec3::ZERO, BRUSH_RADIUS, 1.0), BrushMode::Smooth);
        assert!(
            !outcome.topology_changed(),
            "an already-fine mesh must not be densified"
        );
        outcome_touched += outcome.touched_vertices.len();
    }
    assert_eq!(session.vertex_count(), session.prepared_vertex_count());
    assert!(outcome_touched > 0, "the stroke must still smooth");
}

#[test]
fn add_and_remove_never_change_topology() {
    let mesh = coarse_tent();
    let mut session = BrushSession::prepare(&mesh).expect("prepare");
    for mode in [BrushMode::Add, BrushMode::Remove] {
        for _ in 0..10 {
            let outcome = session.apply_stroke(stroke_at(CREASE_CENTER, BRUSH_RADIUS, 1.0), mode);
            assert!(!outcome.topology_changed(), "{mode:?} must not densify");
        }
    }
    assert_eq!(session.vertex_count(), mesh.vertices.len());
    assert_eq!(session.indices().len(), mesh.indices.len());
}

#[test]
fn densification_can_be_switched_off() {
    let mesh = coarse_tent();
    let mut session = BrushSession::prepare(&mesh).expect("prepare");
    assert!(session.densify_enabled());
    session.set_densify_enabled(false);
    for _ in 0..10 {
        let outcome = session.apply_stroke(
            stroke_at(CREASE_CENTER, BRUSH_RADIUS, 1.0),
            BrushMode::Smooth,
        );
        assert!(!outcome.topology_changed());
    }
    assert_eq!(session.vertex_count(), mesh.vertices.len());
}

#[test]
fn densification_keeps_stl_soup_corners_welded() {
    let indexed = coarse_tent();
    let soup = as_soup(&indexed);
    let mut session = BrushSession::prepare(&soup).expect("prepare");
    for _ in 0..16 {
        session.apply_stroke(
            stroke_at(CREASE_CENTER, BRUSH_RADIUS, 1.0),
            BrushMode::Smooth,
        );
    }
    let after = live_mesh(&session);
    assert!(
        after.vertices.len() > soup.vertices.len(),
        "soup must densify too"
    );
    // Every group of corners that started at one physical point must still
    // share one physical point: a crack would show up as a split cluster.
    let mut clusters: HashMap<[u32; 3], Vec<usize>> = HashMap::new();
    for (index, vertex) in soup.vertices.iter().enumerate() {
        clusters
            .entry(vertex.position.map(f32::to_bits))
            .or_default()
            .push(index);
    }
    for members in clusters.values().filter(|group| group.len() > 1) {
        let reference = Vec3::from_array(after.vertices[members[0]].position);
        for &member in &members[1..] {
            let moved = Vec3::from_array(after.vertices[member].position);
            assert!(
                moved.distance(reference) <= 1e-5,
                "a soup corner cracked apart by {}mm",
                moved.distance(reference)
            );
        }
    }
    // The surface must still be closed the same way it was.
    let histogram = edge_use_histogram(&indexed);
    let after_indexed_like = edge_use_histogram(&after);
    assert!(
        after_indexed_like.keys().max() <= histogram.keys().max().map(|_| &usize::MAX),
        "soup edge histogram must stay sane"
    );
}
