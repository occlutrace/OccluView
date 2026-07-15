//! Boundary-loop discovery and planar ear-clip capping for hole filling.
//!
//! Split out of `holes.rs` (file-size budget): this module owns the low-level
//! rim machinery — turning boundary half-edges into simple loops
//! ([`build_boundary_maps`] + [`walk_boundary_loop`]) and triangulating one
//! loop with a watertight planar ear-clip fan ([`ear_clip_cap`]). The
//! orchestration (gates, refinement, rebuild) stays in `holes.rs`.

use super::adjacency::vertex_index;
use super::{MeshEditBuffers, MeshEditError};
use glam::Vec3;
use std::collections::{HashMap, HashSet};

pub(crate) type BoundaryOwnerMap = HashMap<(usize, usize), usize>;
pub(crate) type BoundaryNextMap = HashMap<usize, usize>;

/// Follow the boundary half-edge chain from `start` until it closes back on
/// itself. Non-simple / broken chains return None; every touched vertex is
/// recorded in `visited` either way so later starts skip it.
pub(crate) fn walk_boundary_loop(
    start: usize,
    next_boundary_vertex: &BoundaryNextMap,
    vertex_count: usize,
    visited: &mut HashSet<usize>,
) -> Option<Vec<usize>> {
    let mut boundary_loop = Vec::new();
    let mut current = start;
    loop {
        if visited.contains(&current) {
            if boundary_loop.first() == Some(&current) {
                return Some(boundary_loop);
            }
            return None;
        }
        visited.insert(current);
        boundary_loop.push(current);
        let &next = next_boundary_vertex.get(&current)?;
        if next == start {
            return Some(boundary_loop);
        }
        current = next;
        if boundary_loop.len() > vertex_count + 1 {
            return None;
        }
    }
}

/// Split a walked boundary loop at coincident-POSITION revisits.
///
/// A hole that touches the scan border (or another hole) at a single vertex
/// is pinch-split per incident fan, which gives the junction one copy per
/// fan — but when the hole's two rim edges at the junction belong to
/// DIFFERENT fans, the walk comes back through the second copy and the hole
/// merges with the border into one long loop. The merged loop then reads as
/// "scan border" and the visibly small hole next to the edge never closes.
///
/// Splitting the loop wherever it passes through two indices carrying the
/// SAME exact position recovers the operator-visible holes: each sub-loop
/// keeps BOTH junction copies (closed by the zero-length virtual edge between
/// them), so its cap pairs every real boundary half-edge of its side and the
/// surface stays closed. A split is taken only when both sides keep at least
/// 3 real edges; unwelded-seam rims where the twin sits right next to its
/// copy stay whole.
pub(crate) fn split_loop_at_coincident_positions(
    mesh: &MeshEditBuffers,
    boundary_loop: Vec<usize>,
) -> Vec<Vec<usize>> {
    let mut pending = vec![boundary_loop];
    let mut finished = Vec::new();
    // Each split leaves both parts strictly shorter in REAL edges than the
    // parent, so the total work is bounded; the guard only protects against
    // a cycle in the presence of NaN-position pathologies.
    let mut rounds = 0_usize;
    while let Some(current) = pending.pop() {
        rounds += 1;
        if rounds > 10_000 {
            finished.push(current);
            continue;
        }
        match first_valid_coincident_pair(mesh, &current) {
            Some((first, second)) => {
                // Inclusive on both sides: each part keeps the coincident
                // pair as its (zero-length) closing edge.
                let inner: Vec<usize> = current[first..=second].to_vec();
                let mut outer: Vec<usize> = current[second..].to_vec();
                outer.extend_from_slice(&current[..=first]);
                pending.push(inner);
                pending.push(outer);
            }
            None => finished.push(current),
        }
    }
    // Deterministic emit order: by first (lowest) original vertex index.
    finished.sort_by_key(|part| part.first().copied().unwrap_or(usize::MAX));
    finished
}

/// The first (lexicographically smallest) index pair `(i, j)` whose vertices
/// carry bitwise-identical positions and whose split keeps 3+ edges on both
/// sides. Sort-based, deterministic.
fn first_valid_coincident_pair(
    mesh: &MeshEditBuffers,
    boundary_loop: &[usize],
) -> Option<(usize, usize)> {
    let len = boundary_loop.len();
    if len < 6 {
        return None; // Both sides need >= 3 edges.
    }
    let mut keyed: Vec<([u32; 3], usize)> = Vec::with_capacity(len);
    for (slot, &vertex) in boundary_loop.iter().enumerate() {
        let position = mesh.vertices.get(vertex)?.position;
        keyed.push((position.map(f32::to_bits), slot));
    }
    keyed.sort_unstable();
    let mut best: Option<(usize, usize)> = None;
    let mut run_start = 0;
    while run_start < keyed.len() {
        let mut run_end = run_start + 1;
        while run_end < keyed.len() && keyed[run_end].0 == keyed[run_start].0 {
            run_end += 1;
        }
        if run_end - run_start > 1 {
            let mut slots: Vec<usize> = keyed[run_start..run_end]
                .iter()
                .map(|&(_, slot)| slot)
                .collect();
            slots.sort_unstable();
            for pair in slots.windows(2) {
                let (first, second) = (pair[0], pair[1]);
                let inner_edges = second - first;
                let outer_edges = len - inner_edges;
                if inner_edges >= 3
                    && outer_edges >= 3
                    && best.is_none_or(|current| (first, second) < current)
                {
                    best = Some((first, second));
                }
            }
        }
        run_start = run_end;
    }
    best
}

pub(crate) fn build_boundary_maps(
    mesh: &MeshEditBuffers,
) -> Result<(BoundaryNextMap, BoundaryOwnerMap, Vec<usize>), MeshEditError> {
    let mut directed_edges = HashSet::with_capacity(mesh.triangle_count() * 3);
    let mut owner_by_edge = HashMap::with_capacity(mesh.triangle_count() * 3);
    for (triangle_index, triangle) in mesh.indices.chunks_exact(3).enumerate() {
        let [a, b, c] = triangle_vertices(triangle, triangle_index)?;
        for edge in [(a, b), (b, c), (c, a)] {
            directed_edges.insert(edge);
            owner_by_edge.insert(edge, triangle_index);
        }
    }

    // Directed edges with no opposing twin are boundary half-edges. A clean
    // rim is a simple directed cycle: every boundary vertex has exactly one
    // outgoing and one incoming boundary half-edge.
    let mut boundary_edges = Vec::new();
    let mut out_degree: HashMap<usize, usize> = HashMap::new();
    let mut in_degree: HashMap<usize, usize> = HashMap::new();
    for &(a, b) in &directed_edges {
        if !directed_edges.contains(&(b, a)) {
            boundary_edges.push((a, b));
            *out_degree.entry(a).or_default() += 1;
            *in_degree.entry(b).or_default() += 1;
        }
    }

    // A vertex where two rims meet (or a non-manifold pinch) has boundary
    // degree > 1. Only vertices with exactly one in- and one out-edge get a
    // unique successor; junction vertices are deliberately LEFT OUT of the
    // successor map so any walk that reaches one dead-ends and is refused,
    // rather than a single overwritten successor silently merging two rims
    // into one figure-eight loop. (Hole filling pre-splits these junctions in
    // `crate::pinch` so they no longer occur on its input.)
    let is_manifold_boundary =
        |vertex: usize| out_degree.get(&vertex) == Some(&1) && in_degree.get(&vertex) == Some(&1);

    let mut next_boundary_vertex = HashMap::new();
    let mut boundary_starts = Vec::new();
    for &(a, b) in &boundary_edges {
        if is_manifold_boundary(a) {
            next_boundary_vertex.insert(a, b);
        }
        // Seed a start from every boundary source, manifold or not: a junction
        // source dead-ends immediately (no successor) and is surfaced as a
        // skipped loop, so pinched rims are never quietly dropped.
        boundary_starts.push(a);
    }
    // `directed_edges` is a HashSet, so iteration order (and thus fill order,
    // triangulation, and appended vertex numbering) is otherwise random per run.
    boundary_starts.sort_unstable();
    boundary_starts.dedup();

    Ok((next_boundary_vertex, owner_by_edge, boundary_starts))
}

fn triangle_vertices(triangle: &[u32], triangle_index: usize) -> Result<[usize; 3], MeshEditError> {
    Ok([
        vertex_index(triangle[0], triangle_index * 3)?,
        vertex_index(triangle[1], triangle_index * 3 + 1)?,
        vertex_index(triangle[2], triangle_index * 3 + 2)?,
    ])
}

/// Rims at or below this length keep the original quadratic ear-clip
/// byte-for-byte (repair's tiny-hole caps stay bit-identical); longer rims
/// take the linked-ring, reflex-aware clipper whose cost is O(n * reflex)
/// instead of O(n^2) full containment scans.
const SMALL_EARCLIP_MAX: usize = 64;

/// Total reflex-containment checks the large ear-clip may spend on one rim.
/// A clean dental rim of 20 000 edges uses a few million; a pathological
/// spiral or a self-intersecting projection would burn quadratic time, so it
/// is refused (deterministically) once the budget is gone.
const LARGE_EARCLIP_WORK_BUDGET: u64 = 50_000_000;

/// Planar ear-clip of one boundary loop, in LOCAL ring indices. The
/// [i0, i2, i1] emit order is the reverse twin of the surrounding side faces'
/// directed boundary edges (watertight winding by construction).
pub(crate) fn ear_clip_cap(
    mesh: &MeshEditBuffers,
    boundary_loop: &[usize],
) -> Result<Vec<[usize; 3]>, MeshEditError> {
    if boundary_loop.len() > SMALL_EARCLIP_MAX {
        return ear_clip_cap_large(mesh, boundary_loop);
    }
    let loop_len = boundary_loop.len();

    let mut centroid = Vec3::ZERO;
    for &vertex_index in boundary_loop {
        centroid += vertex_position(mesh, vertex_index)?;
    }
    let loop_len_u16 = u16::try_from(loop_len).map_err(|_| MeshEditError::InvalidOptions {
        reason: format!(
            "boundary loop length {loop_len} exceeds the conservative hole-filling limit"
        ),
    })?;
    centroid /= f32::from(loop_len_u16);

    let mut normal = Vec3::ZERO;
    for index in 0..loop_len {
        let current = vertex_position(mesh, boundary_loop[index])?;
        let next = vertex_position(mesh, boundary_loop[(index + 1) % loop_len])?;
        normal.x += (current.y - next.y) * (current.z + next.z);
        normal.y += (current.z - next.z) * (current.x + next.x);
        normal.z += (current.x - next.x) * (current.y + next.y);
    }

    if !normal.is_finite() || normal.length_squared() <= f32::EPSILON {
        return Ok(Vec::new());
    }
    let normal = normal.normalize();
    let (u, v) = basis_from_normal(normal);

    let projected: Vec<[f32; 2]> = boundary_loop
        .iter()
        .map(|&vertex_index| {
            let relative = vertex_position(mesh, vertex_index).map(|position| position - centroid);
            relative.map(|relative| [relative.dot(u), relative.dot(v)])
        })
        .collect::<Result<_, _>>()?;

    let mut cap_triangles: Vec<[usize; 3]> = Vec::new();
    let mut ring: Vec<usize> = (0..loop_len).collect();
    let mut guard = 0;
    while ring.len() > 3 && guard < 10 * loop_len {
        guard += 1;
        let ring_len = ring.len();
        let mut clipped = false;
        for index in 0..ring_len {
            let i0 = ring[(index + ring_len - 1) % ring_len];
            let i1 = ring[index];
            let i2 = ring[(index + 1) % ring_len];
            if is_ear(&projected, &ring, i0, i1, i2) {
                cap_triangles.push([i0, i2, i1]);
                ring.remove(index);
                clipped = true;
                break;
            }
        }
        if !clipped {
            break;
        }
    }
    if ring.len() == 3 {
        cap_triangles.push([ring[0], ring[2], ring[1]]);
    }

    // A simple polygon of N vertices triangulates to exactly N - 2 faces. If
    // ear-clipping stalled (self-intersecting / numerically degenerate rim),
    // the fan is incomplete: refuse it wholesale rather than emit a partial,
    // non-watertight cap. The caller records this as a skipped loop.
    if cap_triangles.len() != loop_len - 2 {
        return Ok(Vec::new());
    }
    Ok(cap_triangles)
}

pub(crate) fn push_cap_index(
    new_indices: &mut Vec<u32>,
    global: usize,
) -> Result<(), MeshEditError> {
    new_indices.push(
        u32::try_from(global).map_err(|_| MeshEditError::MalformedMesh {
            reason: "boundary loop vertex index exceeds u32::MAX".to_string(),
        })?,
    );
    Ok(())
}

pub(crate) fn vertex_position(
    mesh: &MeshEditBuffers,
    vertex_index: usize,
) -> Result<Vec3, MeshEditError> {
    mesh.vertices
        .get(vertex_index)
        .map(|vertex| Vec3::from_array(vertex.position))
        .ok_or_else(|| MeshEditError::MalformedMesh {
            reason: format!(
                "boundary loop vertex index {vertex_index} is out of range for vertex_count {}",
                mesh.vertices.len()
            ),
        })
}

fn basis_from_normal(normal: Vec3) -> (Vec3, Vec3) {
    let axis = if normal.x.abs() > 0.9 {
        Vec3::Y
    } else {
        Vec3::X
    };
    let u = axis.cross(normal).normalize();
    let v = normal.cross(u);
    (u, v)
}

/// Ear-clip for rims longer than [`SMALL_EARCLIP_MAX`]. Same contract as the
/// small path (full `n - 2` fan or nothing, same winding), different cost
/// model: a doubly linked ring instead of `Vec::remove`, and containment
/// tested only against REFLEX vertices (a vertex strictly inside a candidate
/// ear is always reflex; clipping only shrinks interior angles, so the reflex
/// set never grows and is kept as a lazily compacted list). The previous full
/// scan made a 20 000-edge rim quadratic — minutes instead of milliseconds.
///
/// Rims longer than `u16::MAX` are refused as a skip (empty result), not an
/// error: such a rim is beyond any cap policy, and one absurd loop must not
/// abort the whole fill run.
fn ear_clip_cap_large(
    mesh: &MeshEditBuffers,
    boundary_loop: &[usize],
) -> Result<Vec<[usize; 3]>, MeshEditError> {
    let loop_len = boundary_loop.len();
    if u16::try_from(loop_len).is_err() {
        return Ok(Vec::new());
    }

    // Centroid in f64 (tens of thousands of f32 sums would drift).
    let mut centroid_sum = glam::DVec3::ZERO;
    for &vertex_index in boundary_loop {
        centroid_sum += vertex_position(mesh, vertex_index)?.as_dvec3();
    }
    let centroid = (centroid_sum / centroid_denominator(loop_len)).as_vec3();

    // Newell normal relative to the centroid (translation-invariant exactly;
    // centering avoids f32 cancellation for far-from-origin rims).
    let mut normal = Vec3::ZERO;
    for index in 0..loop_len {
        let current = vertex_position(mesh, boundary_loop[index])? - centroid;
        let next = vertex_position(mesh, boundary_loop[(index + 1) % loop_len])? - centroid;
        normal.x += (current.y - next.y) * (current.z + next.z);
        normal.y += (current.z - next.z) * (current.x + next.x);
        normal.z += (current.x - next.x) * (current.y + next.y);
    }
    if !normal.is_finite() || normal.length_squared() <= f32::EPSILON {
        return Ok(Vec::new());
    }
    let normal = normal.normalize();
    let (u, v) = basis_from_normal(normal);

    let projected: Vec<[f32; 2]> = boundary_loop
        .iter()
        .map(|&vertex_index| {
            let relative = vertex_position(mesh, vertex_index).map(|position| position - centroid);
            relative.map(|relative| [relative.dot(u), relative.dot(v)])
        })
        .collect::<Result<_, _>>()?;

    let mut ring = LinkedRing::new(loop_len);
    let mut reflex_list: Vec<usize> = Vec::new();
    let mut is_reflex = vec![false; loop_len];
    for (index, flag) in is_reflex.iter_mut().enumerate() {
        if cross_z(&projected, ring.prev[index], index, ring.next[index]) <= 0.0 {
            *flag = true;
            reflex_list.push(index);
        }
    }

    let mut cap_triangles: Vec<[usize; 3]> = Vec::with_capacity(loop_len - 2);
    let mut alive = loop_len;
    let mut current = 0_usize;
    // Global WORK bound (reflex containment checks): a clean rim needs a few
    // hundred per clip at most; a stalled rim (self-intersecting projection)
    // or a pathological spiral gives up deterministically and is refused
    // instead of running for minutes.
    let mut work_left = LARGE_EARCLIP_WORK_BUDGET;
    while alive > 3 {
        let mut scanned = 0_usize;
        let mut clipped = false;
        while scanned < alive && work_left > 0 {
            let i0 = ring.prev[current];
            let i2 = ring.next[current];
            if is_ear_reflex(
                &projected,
                &reflex_list,
                &ring,
                [i0, current, i2],
                &mut work_left,
            ) {
                cap_triangles.push([i0, i2, current]);
                ring.unlink(current);
                is_reflex[current] = false;
                alive -= 1;
                // Neighbors' interior angles only shrink: reflex may turn
                // convex, never the other way. Recheck both.
                for neighbor in [i0, i2] {
                    if is_reflex[neighbor]
                        && cross_z(
                            &projected,
                            ring.prev[neighbor],
                            neighbor,
                            ring.next[neighbor],
                        ) > 0.0
                    {
                        is_reflex[neighbor] = false;
                    }
                }
                reflex_list.retain(|&index| is_reflex[index]);
                // BALANCED peeling: skip one vertex before the next clip.
                // Resuming at the neighbor would clip consecutive corners and
                // rebuild a fan of ~n near-parallel chords 1 rim-edge apart —
                // on a near-cocircular rim the Delaunay predicate ties (no
                // flips fix it up) and density refinement of parallel chords
                // explodes. Skipping alternates the clips so chord lengths
                // double per ring lap: a logarithmic, refinement-friendly cap.
                current = ring.next[i2];
                clipped = true;
                break;
            }
            current = i2;
            scanned += 1;
        }
        if !clipped {
            break;
        }
    }
    if alive == 3 {
        let i1 = current;
        let (i0, i2) = (ring.prev[i1], ring.next[i1]);
        cap_triangles.push([i0, i2, i1]);
    }

    if cap_triangles.len() != loop_len - 2 {
        return Ok(Vec::new());
    }
    Ok(cap_triangles)
}

/// Loop length as an exact f64 divisor (`loop_len` is already `<= u16::MAX`).
fn centroid_denominator(loop_len: usize) -> f64 {
    f64::from(u16::try_from(loop_len).unwrap_or(u16::MAX))
}

/// Doubly linked ring over `0..len` with O(1) unlink.
struct LinkedRing {
    prev: Vec<usize>,
    next: Vec<usize>,
}

impl LinkedRing {
    fn new(len: usize) -> Self {
        let prev = (0..len).map(|i| (i + len - 1) % len).collect();
        let next = (0..len).map(|i| (i + 1) % len).collect();
        Self { prev, next }
    }

    fn unlink(&mut self, index: usize) {
        let (p, n) = (self.prev[index], self.next[index]);
        self.next[p] = n;
        self.prev[n] = p;
    }
}

/// 2D winding cross product at `i1` (positive = convex for a CCW ring).
fn cross_z(projected: &[[f32; 2]], i0: usize, i1: usize, i2: usize) -> f32 {
    let a = projected[i0];
    let b = projected[i1];
    let c = projected[i2];
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

/// Ear test against the reflex set only (a vertex strictly inside a candidate
/// ear of a simple polygon is always reflex). Each containment check consumes
/// one unit of `work_left`; on exhaustion the candidate is rejected, which
/// stalls the clip loop and refuses the rim.
fn is_ear_reflex(
    projected: &[[f32; 2]],
    reflex_list: &[usize],
    ring: &LinkedRing,
    corners: [usize; 3],
    work_left: &mut u64,
) -> bool {
    let [i0, i1, i2] = corners;
    if cross_z(projected, i0, i1, i2) <= 0.0 {
        return false;
    }
    let a = projected[i0];
    let b = projected[i1];
    let c = projected[i2];
    for &index in reflex_list {
        if *work_left == 0 {
            return false;
        }
        *work_left -= 1;
        if index == i0 || index == i1 || index == i2 {
            continue;
        }
        // The caller compacts the list after each clip, so stale entries are
        // at most the vertex clipped since; its links no longer target it.
        if ring.next[ring.prev[index]] != index {
            continue;
        }
        if point_in_triangle(projected[index], a, b, c) {
            return false;
        }
    }
    true
}

fn is_ear(projected: &[[f32; 2]], ring: &[usize], i0: usize, i1: usize, i2: usize) -> bool {
    let a = projected[i0];
    let b = projected[i1];
    let c = projected[i2];
    let cross_z = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
    if cross_z <= 0.0 {
        return false;
    }
    for &index in ring {
        if index == i0 || index == i1 || index == i2 {
            continue;
        }
        if point_in_triangle(projected[index], a, b, c) {
            return false;
        }
    }
    true
}

fn point_in_triangle(p: [f32; 2], a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> bool {
    let d1 = sign2(p, a, b);
    let d2 = sign2(p, b, c);
    let d3 = sign2(p, c, a);
    let has_neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let has_pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(has_neg && has_pos)
}

fn sign2(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    (p[0] - b[0]) * (a[1] - b[1]) - (a[0] - b[0]) * (p[1] - b[1])
}
