//! Rim pre-cleaning: heal the cut line before the hole-fill walk.
//!
//! A lasso deletion (digitally extracting a tooth, then closing the socket)
//! leaves a jagged boundary: lone triangles attached only at a point, needle
//! slivers dangling off the rim by one edge, and near-duplicate seam vertices.
//! Each of those turns into a tiny non-simple loop the cap walk refuses as
//! "damaged", so the operator sees dozens of skipped rims next to the one
//! socket they actually wanted closed.
//!
//! This pass is what makes exocad's close look "smart": it heals the cut line
//! first — removing dangling needle/lone triangles to a fixpoint and welding
//! near-coincident boundary vertices — so the surviving rims are clean simple
//! loops that cap cleanly. It is OPT-IN ([`MeshEditOptions::heal_boundary_rims`],
//! set by the Close Holes path); the repair pipeline leaves it off and stays
//! byte-for-byte unchanged.
//!
//! Every step is topology-only (drop triangles, weld vertex ids); no position
//! ever moves, and the whole pass is deterministic (sorted scans, no hash-order
//! output dependence).

use std::collections::HashMap;

use glam::Vec3;

use super::MeshEditBuffers;

/// A triangle with `>= 2` boundary edges is removed as a needle when its
/// shortest altitude is below this fraction of its longest edge — i.e. it is a
/// sliver too thin to carry surface. A well-shaped rim-corner triangle sits far
/// above this and is kept.
const NEEDLE_ALTITUDE_FRACTION: f32 = 0.02;

/// Near-duplicate boundary vertices closer than this fraction of the median
/// boundary edge length are welded to one id (unwelded scan seam). Kept small
/// so only genuine coincident-seam duplicates merge, never distinct rim points.
const RIM_WELD_FRACTION: f32 = 0.05;

/// Outcome of [`heal_boundary_rims`]: the healed buffers plus the per-original
/// triangle keep-mask (so a face selection can be remapped onto the survivors)
/// and the count of healed defects for the report.
pub(crate) struct RimHealOutcome {
    /// Healed mesh (fewer triangles, possibly fewer distinct vertex ids).
    pub(crate) mesh: MeshEditBuffers,
    /// `true` for each ORIGINAL triangle that survived, in original order.
    pub(crate) keep: Vec<bool>,
    /// Dangling needle/lone triangles removed, plus boundary vertices welded.
    pub(crate) healed: usize,
}

impl RimHealOutcome {
    /// Project a face selection keyed by original triangle index onto the
    /// healed triangle order (dropped triangles' entries fall away).
    pub(crate) fn remap_selection(&self, selection: &super::FaceSelection) -> super::FaceSelection {
        let mask = selection.as_slice();
        let remapped: Vec<bool> = self
            .keep
            .iter()
            .enumerate()
            .filter(|(_, &kept)| kept)
            .map(|(triangle, _)| mask.get(triangle).copied().unwrap_or(false))
            .collect();
        super::FaceSelection::new(remapped)
    }
}

/// Heal dangling needle/lone triangles and near-duplicate boundary vertices.
///
/// Returns `None` when nothing needed healing (the mesh passes through the
/// caller unchanged, byte-for-byte). Otherwise returns the cleaned buffers, the
/// survivor keep-mask, and the healed-defect count. The input's vertex indices
/// are assumed already validated by the caller, so this never fails.
pub(crate) fn heal_boundary_rims(mesh: &MeshEditBuffers) -> Option<RimHealOutcome> {
    let triangle_count = mesh.triangle_count();
    if triangle_count == 0 {
        return None;
    }

    // Phase 1: iteratively drop dangling needle/lone triangles to a fixpoint.
    // Each triangle can be removed at most once, so `triangle_count` rounds is
    // a hard upper bound; in practice a jagged cut converges in a handful.
    let mut alive = vec![true; triangle_count];
    let mut removed_triangles = 0_usize;
    let mut rounds = 0_usize;
    loop {
        rounds += 1;
        if rounds > triangle_count + 1 {
            break;
        }
        let doomed = dangling_triangles(mesh, &alive);
        if doomed.is_empty() {
            break;
        }
        for triangle in doomed {
            alive[triangle] = false;
            removed_triangles += 1;
        }
    }

    // Phase 2: weld near-duplicate boundary vertices among the survivors.
    let (vertex_remap, welded) = weld_boundary_vertices(mesh, &alive);

    let healed = removed_triangles + welded;
    if healed == 0 {
        return None;
    }

    // Rebuild survivor indices, applying the weld remap, dropping any triangle
    // that the weld collapsed to a degenerate (two ids equal).
    let mut indices: Vec<u32> = Vec::with_capacity(mesh.indices.len());
    let mut keep = vec![false; triangle_count];
    for (triangle, tri) in mesh.indices.chunks_exact(3).enumerate() {
        if !alive[triangle] {
            continue;
        }
        let a = vertex_remap[tri[0] as usize];
        let b = vertex_remap[tri[1] as usize];
        let c = vertex_remap[tri[2] as usize];
        if a == b || b == c || c == a {
            // Weld collapsed this face; count it as healed and drop it.
            continue;
        }
        keep[triangle] = true;
        indices.push(a);
        indices.push(b);
        indices.push(c);
    }

    Some(RimHealOutcome {
        mesh: MeshEditBuffers {
            vertices: mesh.vertices.clone(),
            indices,
            topology: mesh.topology,
        },
        keep,
        healed,
    })
}

/// Directed-edge multiset over the ALIVE triangles: an edge with no opposing
/// twin is a boundary half-edge.
fn boundary_edge_set(
    mesh: &MeshEditBuffers,
    alive: &[bool],
) -> std::collections::HashSet<(u32, u32)> {
    let mut directed: std::collections::HashSet<(u32, u32)> =
        std::collections::HashSet::with_capacity(mesh.indices.len());
    for (triangle, tri) in mesh.indices.chunks_exact(3).enumerate() {
        if !alive[triangle] {
            continue;
        }
        directed.insert((tri[0], tri[1]));
        directed.insert((tri[1], tri[2]));
        directed.insert((tri[2], tri[0]));
    }
    directed
}

/// Alive triangles that are dangling debris: either isolated (all three edges
/// on the boundary) or a needle sliver hanging off the rim by one edge (two
/// boundary edges and a near-zero shortest altitude). Deterministic ascending
/// triangle order.
fn dangling_triangles(mesh: &MeshEditBuffers, alive: &[bool]) -> Vec<usize> {
    let directed = boundary_edge_set(mesh, alive);
    let is_boundary = |a: u32, b: u32| !directed.contains(&(b, a));

    let mut doomed = Vec::new();
    for (triangle, tri) in mesh.indices.chunks_exact(3).enumerate() {
        if !alive[triangle] {
            continue;
        }
        let edges = [(tri[0], tri[1]), (tri[1], tri[2]), (tri[2], tri[0])];
        let boundary_edges = edges.iter().filter(|&&(a, b)| is_boundary(a, b)).count();
        if boundary_edges >= 3 {
            // Isolated triangle: no watertight cap exists (its only cover is
            // its own reverse twin). Always debris.
            doomed.push(triangle);
        } else if boundary_edges == 2 && is_needle(mesh, tri) {
            // Sliver dangling off the rim by a single shared edge.
            doomed.push(triangle);
        }
    }
    doomed
}

/// Whether a triangle is a needle sliver: its shortest altitude is a tiny
/// fraction of its longest edge (equivalently, near-zero area for its extent).
fn is_needle(mesh: &MeshEditBuffers, tri: &[u32]) -> bool {
    let p: [Vec3; 3] = [
        Vec3::from_array(mesh.vertices[tri[0] as usize].position),
        Vec3::from_array(mesh.vertices[tri[1] as usize].position),
        Vec3::from_array(mesh.vertices[tri[2] as usize].position),
    ];
    let longest = (p[1] - p[0])
        .length()
        .max((p[2] - p[1]).length())
        .max((p[0] - p[2]).length());
    if longest.is_finite() && longest > 0.0 {
        let area = (p[1] - p[0]).cross(p[2] - p[0]).length() * 0.5;
        let shortest_altitude = 2.0 * area / longest;
        shortest_altitude < NEEDLE_ALTITUDE_FRACTION * longest
    } else {
        true // Zero-extent / non-finite debris.
    }
}

/// Weld near-coincident boundary vertices among the alive triangles to a single
/// id. Returns a full vertex remap (`old id -> canonical id`) and the number of
/// vertices merged away. Non-boundary vertices always map to themselves.
///
/// Deterministic: candidates are bucketed on a quantized grid, and each cluster
/// elects its LOWEST id as canonical after a sorted union.
fn weld_boundary_vertices(mesh: &MeshEditBuffers, alive: &[bool]) -> (Vec<u32>, usize) {
    let vertex_count = mesh.vertices.len();
    // Identity remap over vertex ids (all referenced ids fit u32 by validation).
    let identity = || -> Vec<u32> {
        (0..vertex_count)
            .map(|i| u32::try_from(i).unwrap_or(u32::MAX))
            .collect()
    };
    let mut remap: Vec<u32> = identity();

    // Boundary vertices of the alive sub-mesh, and the median boundary edge
    // length that sets the weld tolerance.
    let directed = boundary_edge_set(mesh, alive);
    let mut boundary: Vec<u32> = Vec::new();
    let mut seen = vec![false; vertex_count];
    let mut edge_lengths: Vec<f32> = Vec::new();
    for &(a, b) in &directed {
        if directed.contains(&(b, a)) {
            continue;
        }
        for v in [a, b] {
            if !seen[v as usize] {
                seen[v as usize] = true;
                boundary.push(v);
            }
        }
        let pa = Vec3::from_array(mesh.vertices[a as usize].position);
        let pb = Vec3::from_array(mesh.vertices[b as usize].position);
        edge_lengths.push((pb - pa).length());
    }
    if boundary.len() < 2 || edge_lengths.is_empty() {
        return (remap, 0);
    }
    boundary.sort_unstable();
    edge_lengths.sort_by(|l, r| l.partial_cmp(r).unwrap_or(std::cmp::Ordering::Equal));
    let median = edge_lengths[edge_lengths.len() / 2];
    let tol = median * RIM_WELD_FRACTION;
    if !tol.is_finite() || tol <= 0.0 {
        return (remap, 0);
    }

    // Bucket boundary vertices on a grid of side `tol`; only same/adjacent
    // buckets can be within tolerance. Deterministic: buckets keyed by integer
    // cell, members in ascending id.
    let mut grid: HashMap<[i64; 3], Vec<u32>> = HashMap::new();
    for &v in &boundary {
        let p = Vec3::from_array(mesh.vertices[v as usize].position);
        grid.entry(cell_of(p, tol)).or_default().push(v);
    }

    // Union-find over boundary vertices (indexed by their id directly).
    let mut parent: Vec<u32> = identity();
    let tol_sq = tol * tol;
    for &v in &boundary {
        let pv = Vec3::from_array(mesh.vertices[v as usize].position);
        let base = cell_of(pv, tol);
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    let key = [base[0] + dx, base[1] + dy, base[2] + dz];
                    let Some(members) = grid.get(&key) else {
                        continue;
                    };
                    for &w in members {
                        if w <= v {
                            continue;
                        }
                        let pw = Vec3::from_array(mesh.vertices[w as usize].position);
                        // Coincident attribute-distinct duplicates (a color/uv
                        // seam) fuse here TOO, on purpose: a seam slit is an
                        // index-space boundary, and leaving it open would feed
                        // the hole filler two giant phantom rims to cap with
                        // membranes. Closing the topology wins; the cost is a
                        // one-vertex-wide color blend along healed rims only.
                        if (pw - pv).length_squared() <= tol_sq {
                            union(&mut parent, v, w);
                        }
                    }
                }
            }
        }
    }

    // Canonicalize each vertex to its cluster's lowest id.
    let mut welded = 0_usize;
    for &v in &boundary {
        let root = find(&mut parent, v);
        if root != v {
            remap[v as usize] = root;
            welded += 1;
        }
    }
    (remap, welded)
}

/// Integer grid cell of a position at side `tol`. The coordinates of a dental
/// mesh (millimetres, bounded to a few hundred) divided by a positive `tol`
/// land far inside `i64`, so the post-`floor` narrowing cast cannot overflow.
#[allow(clippy::cast_possible_truncation)]
fn cell_of(p: Vec3, tol: f32) -> [i64; 3] {
    [
        (p.x / tol).floor() as i64,
        (p.y / tol).floor() as i64,
        (p.z / tol).floor() as i64,
    ]
}

fn find(parent: &mut [u32], mut node: u32) -> u32 {
    while parent[node as usize] != node {
        parent[node as usize] = parent[parent[node as usize] as usize];
        node = parent[node as usize];
    }
    node
}

fn union(parent: &mut [u32], a: u32, b: u32) {
    let ra = find(parent, a);
    let rb = find(parent, b);
    if ra != rb {
        let (low, high) = (ra.min(rb), ra.max(rb));
        parent[high as usize] = low;
    }
}
