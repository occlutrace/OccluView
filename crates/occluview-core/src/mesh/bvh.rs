//! A compact triangle bounding-volume hierarchy for ray picking.
//!
//! Scene picking was brute-force O(triangles): fine for the occasional focus
//! click, ruinous for a sculpt brush that raycasts the mesh every hover and drag
//! frame — on a million-triangle scan that is tens of milliseconds per frame
//! twice over (cursor + dab). This BVH turns a pick into O(log n). It is built
//! LAZILY (the first pick pays for it, in parallel) and cached on the `Mesh`;
//! because mesh geometry is immutable after construction, the cache never goes
//! stale — a sculpt commit mints a new `Mesh`, which builds its own.

// Triangle/vertex counts never approach u32::MAX, so the index casts here are
// exact; the ray-math functions genuinely want short local names.
#![allow(clippy::cast_possible_truncation, clippy::many_single_char_names)]

use super::Vertex;
use glam::Vec3;
use rayon::prelude::*;

/// Triangles per leaf. Small enough that a leaf test is cheap, large enough to
/// keep the tree shallow.
const LEAF_MAX: usize = 8;

/// One triangle hit in mesh-local space.
pub(crate) struct BvhHit {
    /// Triangle index (chunk index into the mesh's `indices`).
    pub(crate) triangle_index: usize,
    /// Hit position in mesh-local space.
    pub(crate) point: Vec3,
    /// Ray parameter (distance along the unit local direction).
    pub(crate) distance: f32,
}

#[derive(Clone, Copy, Debug)]
struct Node {
    min: Vec3,
    max: Vec3,
    /// Leaf: start offset into `order`. Internal: left child node index.
    left_or_start: u32,
    /// Leaf: triangle count (> 0). Internal: right child node index, `is_leaf`
    /// tells them apart.
    right_or_count: u32,
    is_leaf: bool,
}

/// A median-split triangle BVH over one mesh's LOCAL-space triangles.
#[derive(Clone, Debug)]
pub(crate) struct TriangleBvh {
    nodes: Vec<Node>,
    /// Triangle indices in leaf order.
    order: Vec<u32>,
}

impl TriangleBvh {
    /// Build over `indices` (triangle list) and `vertices` (local positions).
    pub(crate) fn build(vertices: &[Vertex], indices: &[u32]) -> Self {
        let triangle_count = indices.len() / 3;
        // Per-triangle bounds + centroid, in parallel — the embarrassingly
        // parallel part of the build.
        let bounds: Vec<TriBounds> = (0..triangle_count)
            .into_par_iter()
            .map(|triangle| {
                let base = triangle * 3;
                let a = position(vertices, indices[base]);
                let b = position(vertices, indices[base + 1]);
                let c = position(vertices, indices[base + 2]);
                TriBounds {
                    min: a.min(b).min(c),
                    max: a.max(b).max(c),
                    centroid: (a + b + c) / 3.0,
                }
            })
            .collect();

        let mut order: Vec<u32> = (0..triangle_count as u32).collect();
        let mut nodes: Vec<Node> = Vec::with_capacity(triangle_count.max(1) * 2);
        if triangle_count > 0 {
            build_recursive(&mut nodes, &mut order, &bounds, 0, triangle_count);
        }
        Self { nodes, order }
    }

    /// Recompute only the node bounds after vertex positions changed while
    /// triangle topology stayed the same. Sculpt commits use this instead of
    /// throwing away the acceleration structure and rebuilding/sorting every
    /// triangle. The tree shape and leaf order remain valid; only the boxes
    /// need to follow the edited vertices.
    pub(crate) fn refit(&mut self, vertices: &[Vertex], indices: &[u32]) {
        for node_index in (0..self.nodes.len()).rev() {
            let node = self.nodes[node_index];
            let (min, max) = if node.is_leaf {
                let start = node.left_or_start as usize;
                let end = start + node.right_or_count as usize;
                let mut bounds: Option<(Vec3, Vec3)> = None;
                for &triangle in &self.order[start..end] {
                    let base = triangle as usize * 3;
                    let Some(corners) = indices.get(base..base + 3) else {
                        continue;
                    };
                    let a = position(vertices, corners[0]);
                    let b = position(vertices, corners[1]);
                    let c = position(vertices, corners[2]);
                    let triangle_min = a.min(b).min(c);
                    let triangle_max = a.max(b).max(c);
                    bounds = Some(match bounds {
                        Some((min, max)) => (min.min(triangle_min), max.max(triangle_max)),
                        None => (triangle_min, triangle_max),
                    });
                }
                bounds.unwrap_or((Vec3::ZERO, Vec3::ZERO))
            } else {
                let left = self.nodes[node.left_or_start as usize];
                let right = self.nodes[node.right_or_count as usize];
                (left.min.min(right.min), left.max.max(right.max))
            };
            self.nodes[node_index].min = min;
            self.nodes[node_index].max = max;
        }
    }

    /// Nearest triangle hit of the LOCAL ray `origin + t·direction` whose local
    /// point satisfies `keep`. `direction` need not be unit; the returned
    /// distance is along the normalized direction.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn pick<K>(
        &self,
        vertices: &[Vertex],
        indices: &[u32],
        origin: Vec3,
        direction: Vec3,
        keep: K,
    ) -> Option<BvhHit>
    where
        K: Fn(Vec3) -> bool,
    {
        let direction = direction.normalize_or_zero();
        if self.nodes.is_empty() || direction.length_squared() <= f32::EPSILON {
            return None;
        }
        let inv_dir = Vec3::new(
            safe_inv(direction.x),
            safe_inv(direction.y),
            safe_inv(direction.z),
        );

        let mut best: Option<BvhHit> = None;
        let mut stack = [0u32; 64];
        let mut depth = 1usize;
        stack[0] = 0;
        while depth > 0 {
            depth -= 1;
            let node = self.nodes[stack[depth] as usize];
            let Some(entry) = ray_aabb(origin, inv_dir, node.min, node.max) else {
                continue;
            };
            if best.as_ref().is_some_and(|hit| entry > hit.distance) {
                continue;
            }
            if node.is_leaf {
                let start = node.left_or_start as usize;
                let end = start + node.right_or_count as usize;
                for &triangle in &self.order[start..end] {
                    let base = triangle as usize * 3;
                    let a = position(vertices, indices[base]);
                    let b = position(vertices, indices[base + 1]);
                    let c = position(vertices, indices[base + 2]);
                    if let Some((distance, point)) = ray_triangle(origin, direction, a, b, c) {
                        let nearer = best.as_ref().is_none_or(|hit| distance < hit.distance);
                        if nearer && keep(point) {
                            best = Some(BvhHit {
                                triangle_index: triangle as usize,
                                point,
                                distance,
                            });
                        }
                    }
                }
            } else if depth + 2 <= stack.len() {
                stack[depth] = node.left_or_start;
                stack[depth + 1] = node.right_or_count;
                depth += 2;
            }
        }
        best
    }
}

struct TriBounds {
    min: Vec3,
    max: Vec3,
    centroid: Vec3,
}

/// Build the subtree for `order[start..end]`, returning its node index.
fn build_recursive(
    nodes: &mut Vec<Node>,
    order: &mut [u32],
    bounds: &[TriBounds],
    start: usize,
    end: usize,
) -> u32 {
    let node_index = nodes.len() as u32;
    nodes.push(Node {
        min: Vec3::ZERO,
        max: Vec3::ZERO,
        left_or_start: 0,
        right_or_count: 0,
        is_leaf: true,
    });

    let (mut min, mut max) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    let (mut cmin, mut cmax) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for &triangle in &order[start..end] {
        let tri = &bounds[triangle as usize];
        min = min.min(tri.min);
        max = max.max(tri.max);
        cmin = cmin.min(tri.centroid);
        cmax = cmax.max(tri.centroid);
    }

    let count = end - start;
    if count <= LEAF_MAX {
        nodes[node_index as usize] = Node {
            min,
            max,
            left_or_start: start as u32,
            right_or_count: count as u32,
            is_leaf: true,
        };
        return node_index;
    }

    // Split on the axis of greatest centroid spread, at the median centroid.
    let extent = cmax - cmin;
    let axis = if extent.x >= extent.y && extent.x >= extent.z {
        0
    } else if extent.y >= extent.z {
        1
    } else {
        2
    };
    let median = start + count / 2;
    order[start..end].select_nth_unstable_by(count / 2, |&l, &r| {
        bounds[l as usize].centroid[axis].total_cmp(&bounds[r as usize].centroid[axis])
    });

    let left = build_recursive(nodes, order, bounds, start, median);
    let right = build_recursive(nodes, order, bounds, median, end);
    nodes[node_index as usize] = Node {
        min,
        max,
        left_or_start: left,
        right_or_count: right,
        is_leaf: false,
    };
    node_index
}

fn position(vertices: &[Vertex], raw: u32) -> Vec3 {
    Vec3::from_array(vertices[raw as usize].position)
}

/// Reciprocal of a ray-direction component. For an axis-parallel component
/// (near zero) it returns a large FINITE value rather than infinity: `INFINITY`
/// would make `0 · ∞ = NaN` in the slab test whenever a box face lies exactly on
/// the ray origin's coordinate, silently dropping that node (and any hit inside
/// it). A huge finite reciprocal keeps the slab math correct and NaN-free.
fn safe_inv(value: f32) -> f32 {
    const HUGE: f32 = 1.0e30;
    if value.abs() > f32::EPSILON {
        1.0 / value
    } else if value.is_sign_negative() {
        -HUGE
    } else {
        HUGE
    }
}

/// Ray-AABB slab test: the entry `t` (clamped to 0 if the origin is inside), or
/// `None` on a miss.
fn ray_aabb(origin: Vec3, inv_dir: Vec3, min: Vec3, max: Vec3) -> Option<f32> {
    let t0 = (min - origin) * inv_dir;
    let t1 = (max - origin) * inv_dir;
    let tmin = t0.min(t1);
    let tmax = t0.max(t1);
    let enter = tmin.x.max(tmin.y).max(tmin.z);
    let exit = tmax.x.min(tmax.y).min(tmax.z);
    (exit >= enter.max(0.0)).then(|| enter.max(0.0))
}

/// Möller-Trumbore ray-triangle intersection (front and back faces), returning
/// `(distance, point)` for a forward hit.
fn ray_triangle(origin: Vec3, direction: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Option<(f32, Vec3)> {
    const EPSILON: f32 = 1e-6;
    let edge0 = b - a;
    let edge1 = c - a;
    let determinant_cross = direction.cross(edge1);
    let determinant = edge0.dot(determinant_cross);
    if determinant.abs() <= EPSILON {
        return None;
    }
    let inv_determinant = 1.0 / determinant;
    let origin_to_a = origin - a;
    let u = origin_to_a.dot(determinant_cross) * inv_determinant;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let barycentric_cross = origin_to_a.cross(edge0);
    let v = direction.dot(barycentric_cross) * inv_determinant;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let distance = edge1.dot(barycentric_cross) * inv_determinant;
    if distance <= EPSILON || !distance.is_finite() {
        return None;
    }
    Some((distance, origin + direction * distance))
}
