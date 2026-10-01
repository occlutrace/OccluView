//! Immutable reference-surface probe used by the Remove wall reserve.
//!
//! A small CSR triangle grid lets a ray walk only the cells between a sculpted
//! point and its opposing wall. The probe is prepared once per session, then
//! readings are memoized by group as the brush reaches them.

use glam::DVec3;

const SELF_SKIN_MM: f64 = 0.02;
const GOLDEN_ANGLE: f64 = 2.399_963_229_728_653;
const OPPOSING_WALL_COS: f64 = 0.5;
const PROBE_CELL_MM: f64 = 0.3;
const MAX_PROBE_CELLS: f64 = 16_000_000.0;

fn reference_bounds(verts: &[f32]) -> (DVec3, DVec3) {
    let mut lo = DVec3::splat(f64::INFINITY);
    let mut hi = DVec3::splat(f64::NEG_INFINITY);
    for &[x, y, z] in verts.as_chunks::<3>().0 {
        let point = DVec3::new(f64::from(x), f64::from(y), f64::from(z));
        if point.is_finite() {
            lo = lo.min(point);
            hi = hi.max(point);
        }
    }
    if lo.x.is_finite() {
        (lo, hi)
    } else {
        (DVec3::ZERO, DVec3::ZERO)
    }
}

fn ray_hit_signed(orig: DVec3, dir: DVec3, a: DVec3, b: DVec3, c: DVec3) -> Option<(f64, bool)> {
    let e1 = b - a;
    let e2 = c - a;
    let p = dir.cross(e2);
    let det = e1.dot(p);
    if (-1e-12..=1e-12).contains(&det) {
        return None;
    }
    let inv = 1.0 / det;
    let s = orig - a;
    let u = s.dot(p) * inv;
    if !(-1e-9..=1.0 + 1e-9).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = dir.dot(q) * inv;
    if v < -1e-9 || u + v > 1.0 + 1e-9 {
        return None;
    }
    let t = e2.dot(q) * inv;
    (t > 1e-9).then_some((t, det < 0.0))
}

/// Prepared ray caster over the immutable session-opening mesh.
pub(crate) struct SdfProbe {
    verts: Vec<f32>,
    tris: Vec<u32>,
    max_mm: f64,
    n_rays: usize,
    cone: f64,
    cell: f64,
    lo: DVec3,
    dims: (i32, i32, i32),
    cell_off: Vec<u32>,
    cell_tris: Vec<u32>,
}

impl SdfProbe {
    pub(crate) fn new(
        verts: Vec<f32>,
        tris: Vec<u32>,
        max_mm: f64,
        n_rays: usize,
        cone_deg: f64,
    ) -> Self {
        let vertex_count = verts.len() / 3;
        let triangle_count = tris.len() / 3;
        let point = |vertex: usize| {
            let offset = vertex * 3;
            DVec3::new(
                verts[offset] as f64,
                verts[offset + 1] as f64,
                verts[offset + 2] as f64,
            )
        };
        let (mut lo, hi) = reference_bounds(&verts);

        // Keep the dense lookup bounded on very large models; the DDA still
        // traverses only cells crossed by each ray.
        let span = hi - lo;
        let mut cell = PROBE_CELL_MM;
        while (span.x / cell + 3.0) * (span.y / cell + 3.0) * (span.z / cell + 3.0)
            > MAX_PROBE_CELLS
        {
            cell *= 1.25;
        }
        lo -= DVec3::splat(cell);
        let dims = (
            (((hi.x - lo.x) / cell).floor() as i32) + 2,
            (((hi.y - lo.y) / cell).floor() as i32) + 2,
            (((hi.z - lo.z) / cell).floor() as i32) + 2,
        );
        let cell_count = dims.0 as usize * dims.1 as usize * dims.2 as usize;
        let cell_index = |x: i32, y: i32, z: i32| -> usize {
            (x as usize * dims.1 as usize + y as usize) * dims.2 as usize + z as usize
        };
        let tri_span = |triangle: usize| -> Option<((i32, i32, i32), (i32, i32, i32))> {
            let indices = &tris[triangle * 3..triangle * 3 + 3];
            if indices.iter().any(|&index| index as usize >= vertex_count) {
                return None;
            }
            let (a, b, c) = (
                point(indices[0] as usize),
                point(indices[1] as usize),
                point(indices[2] as usize),
            );
            if !a.is_finite() || !b.is_finite() || !c.is_finite() {
                return None;
            }
            let tri_lo = a.min(b).min(c);
            let tri_hi = a.max(b).max(c);
            Some((
                (
                    (((tri_lo.x - lo.x) / cell).floor() as i32).clamp(0, dims.0 - 1),
                    (((tri_lo.y - lo.y) / cell).floor() as i32).clamp(0, dims.1 - 1),
                    (((tri_lo.z - lo.z) / cell).floor() as i32).clamp(0, dims.2 - 1),
                ),
                (
                    (((tri_hi.x - lo.x) / cell).floor() as i32).clamp(0, dims.0 - 1),
                    (((tri_hi.y - lo.y) / cell).floor() as i32).clamp(0, dims.1 - 1),
                    (((tri_hi.z - lo.z) / cell).floor() as i32).clamp(0, dims.2 - 1),
                ),
            ))
        };
        let mut counts = vec![0u32; cell_count + 1];
        for triangle in 0..triangle_count {
            let Some((from, to)) = tri_span(triangle) else {
                continue;
            };
            for x in from.0..=to.0 {
                for y in from.1..=to.1 {
                    for z in from.2..=to.2 {
                        counts[cell_index(x, y, z) + 1] += 1;
                    }
                }
            }
        }
        for index in 0..cell_count {
            counts[index + 1] += counts[index];
        }
        let mut cursor = counts.clone();
        let mut cell_tris = vec![0u32; counts[cell_count] as usize];
        for triangle in 0..triangle_count {
            let Some((from, to)) = tri_span(triangle) else {
                continue;
            };
            for x in from.0..=to.0 {
                for y in from.1..=to.1 {
                    for z in from.2..=to.2 {
                        let slot = cell_index(x, y, z);
                        cell_tris[cursor[slot] as usize] = triangle as u32;
                        cursor[slot] += 1;
                    }
                }
            }
        }
        Self {
            verts,
            tris,
            max_mm: max_mm.max(0.0),
            n_rays: n_rays.max(1),
            cone: cone_deg.to_radians(),
            cell,
            lo,
            dims,
            cell_off: counts,
            cell_tris,
        }
    }

    fn point(&self, vertex: usize) -> DVec3 {
        let offset = vertex * 3;
        DVec3::new(
            self.verts[offset] as f64,
            self.verts[offset + 1] as f64,
            self.verts[offset + 2] as f64,
        )
    }

    fn cell_range(&self, x: i32, y: i32, z: i32) -> &[u32] {
        if x < 0 || y < 0 || z < 0 || x >= self.dims.0 || y >= self.dims.1 || z >= self.dims.2 {
            return &[];
        }
        let index =
            (x as usize * self.dims.1 as usize + y as usize) * self.dims.2 as usize + z as usize;
        &self.cell_tris[self.cell_off[index] as usize..self.cell_off[index + 1] as usize]
    }

    fn cone_ray(&self, inward: DVec3, u: DVec3, w: DVec3, ray: usize) -> (DVec3, f64) {
        let fraction = if self.n_rays > 1 {
            ray as f64 / (self.n_rays - 1) as f64
        } else {
            0.0
        };
        let angle = self.cone * fraction.sqrt();
        let azimuth = ray as f64 * GOLDEN_ANGLE;
        let (sin, cos) = (angle.sin(), angle.cos());
        (
            inward * cos + u * (azimuth.cos() * sin) + w * (azimuth.sin() * sin),
            cos,
        )
    }

    fn cast(&self, origin: DVec3, dir: DVec3, inward: DVec3, reach: f64) -> Option<f64> {
        let rel = origin - self.lo;
        let mut cell_at = (
            (rel.x / self.cell).floor() as i32,
            (rel.y / self.cell).floor() as i32,
            (rel.z / self.cell).floor() as i32,
        );
        let axis = |position: f64, direction: f64, index: i32| -> (i32, f64, f64) {
            if direction > 0.0 {
                let boundary = (index + 1) as f64 * self.cell;
                (1, (boundary - position) / direction, self.cell / direction)
            } else if direction < 0.0 {
                let boundary = index as f64 * self.cell;
                (
                    -1,
                    (boundary - position) / direction,
                    -self.cell / direction,
                )
            } else {
                (0, f64::INFINITY, f64::INFINITY)
            }
        };
        let (step_x, mut next_x, delta_x) = axis(rel.x, dir.x, cell_at.0);
        let (step_y, mut next_y, delta_y) = axis(rel.y, dir.y, cell_at.1);
        let (step_z, mut next_z, delta_z) = axis(rel.z, dir.z, cell_at.2);
        let mut best = f64::INFINITY;
        let mut best_opposes = false;
        let mut enter = 0.0;
        while enter <= reach && enter <= best {
            for &triangle in self.cell_range(cell_at.0, cell_at.1, cell_at.2) {
                let offset = triangle as usize * 3;
                let (a, b, c) = (
                    self.point(self.tris[offset] as usize),
                    self.point(self.tris[offset + 1] as usize),
                    self.point(self.tris[offset + 2] as usize),
                );
                if let Some((distance, exits)) = ray_hit_signed(origin, dir, a, b, c) {
                    if exits && distance < best {
                        best = distance;
                        let normal = (b - a).cross(c - a);
                        best_opposes = normal.length() > 0.0
                            && normal.dot(inward) >= OPPOSING_WALL_COS * normal.length();
                    }
                }
            }
            if next_x <= next_y && next_x <= next_z {
                enter = next_x;
                next_x += delta_x;
                cell_at.0 += step_x;
            } else if next_y <= next_z {
                enter = next_y;
                next_y += delta_y;
                cell_at.1 += step_y;
            } else {
                enter = next_z;
                next_z += delta_z;
                cell_at.2 += step_z;
            }
            if !enter.is_finite() {
                break;
            }
        }
        (best <= reach && best_opposes).then_some(best)
    }

    fn cone_frame(inward: DVec3) -> (DVec3, DVec3) {
        // Match the donor's `plane_basis`: its deterministic helper axis fixes
        // the golden-angle ray coordinates. Any valid orthonormal frame
        // rotates this finite eight-ray set and can change an irregular wall's
        // median thickness.
        let helper = if inward.x.abs() < 0.9 {
            DVec3::X
        } else {
            DVec3::Y
        };
        let u = inward.cross(helper).normalize_or_zero();
        let w = inward.cross(u).normalize_or_zero();
        (u, w)
    }

    /// Cone-median support thickness at a material position and normal. The
    /// pose is explicit because live remesh can mint ids beyond this snapshot.
    pub(crate) fn thickness_at_pose(&self, origin: DVec3, normal: DVec3) -> f32 {
        let length = normal.length();
        if length < 1e-12 {
            return self.max_mm as f32;
        }
        let inward = -normal / length;
        let (u, w) = Self::cone_frame(inward);
        let mut hits = Vec::with_capacity(self.n_rays);
        let needed = self.n_rays.div_ceil(2);
        for ray in 0..self.n_rays {
            if hits.len() + (self.n_rays - ray) < needed {
                break;
            }
            let (dir, cosine) = self.cone_ray(inward, u, w, ray);
            let ray_origin = origin + dir * SELF_SKIN_MM;
            if let Some(distance) = self.cast(ray_origin, dir, inward, self.max_mm) {
                hits.push(((distance + SELF_SKIN_MM) * cosine.abs()).min(self.max_mm));
            }
        }
        if hits.len() * 2 >= self.n_rays {
            hits.sort_by(f64::total_cmp);
            hits[hits.len() / 2] as f32
        } else {
            self.max_mm as f32
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cone_frame_matches_the_donor_basis_for_irregular_normals() {
        for inward in [
            DVec3::new(0.31, -0.71, 0.63).normalize(),
            DVec3::new(0.94, 0.2, -0.27).normalize(),
        ] {
            let helper = if inward.x.abs() < 0.9 {
                DVec3::X
            } else {
                DVec3::Y
            };
            let expected_u = inward.cross(helper).normalize();
            let expected_w = inward.cross(expected_u).normalize();
            let (u, w) = SdfProbe::cone_frame(inward);
            assert_eq!(u, expected_u);
            assert_eq!(w, expected_w);
            assert!(u.dot(w).abs() < 1e-12);
            assert!(inward.dot(u).abs() < 1e-12);
            assert!(inward.dot(w).abs() < 1e-12);
        }
    }
}
