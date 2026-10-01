use super::DVec3;

/// Interpolate camera near/far endpoints, then derive this segment's length.
pub(super) fn interpolate_visible_segment(
    start: (DVec3, DVec3, f64, f64),
    end: (DVec3, DVec3, f64, f64),
    amount: f64,
) -> (DVec3, DVec3, f64) {
    let (so, sd, sn, sf) = start;
    let (eo, ed, en, ef) = end;
    let near_start = so + sd * sn;
    let near_end = eo + ed * en;
    let far_start = so + sd * sf;
    let far_end = eo + ed * ef;
    let origin = near_start + ((near_end - near_start) * amount);
    let endpoint = far_start + ((far_end - far_start) * amount);
    let segment = endpoint - origin;
    (origin, segment.normalize_or_zero(), segment.length())
}

/// Camera interval and visible mesh-local halfspaces. Invalid input fails closed.
#[derive(Clone, Copy)]
pub struct SculptRayConstraints<'a> {
    /// Nearest permitted distance along the ray.
    pub near: f64,
    /// Farthest permitted distance along the ray.
    pub far: f64,
    /// Visible halfspaces in mesh-local coordinates. Each `[nx, ny, nz, d]`
    /// contains points where `n·p + d >= 0`, using the renderer's keep-side sign.
    pub clip_planes: &'a [[f64; 4]],
}
impl Default for SculptRayConstraints<'_> {
    fn default() -> Self {
        Self {
            near: 0.0,
            far: f64::INFINITY,
            clip_planes: &[],
        }
    }
}
impl SculptRayConstraints<'_> {
    pub(super) fn valid(self, origin: DVec3, dir: DVec3) -> bool {
        [origin.x, origin.y, origin.z, dir.x, dir.y, dir.z]
            .iter()
            .all(|v| v.is_finite())
            && dir.length().is_finite()
            && dir.length() > 1e-12
            && self.near.is_finite()
            && self.near >= 0.0
            && self.far >= self.near
            && self.far != f64::NEG_INFINITY
            && self.clip_planes.iter().all(|p| {
                p.iter().all(|v| v.is_finite())
                    && DVec3::new(p[0], p[1], p[2]).length().is_finite()
                    && DVec3::new(p[0], p[1], p[2]).length() > 0.0
            })
    }
    pub(super) fn contains(self, p: DVec3) -> bool {
        self.clip_planes
            .iter()
            .all(|plane| plane[0] * p.x + plane[1] * p.y + plane[2] * p.z + plane[3] >= 0.0)
    }
}
