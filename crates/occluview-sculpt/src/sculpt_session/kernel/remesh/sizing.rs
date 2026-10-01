use super::*;

/// Area weighting prevents a tiny over-tessellated patch from prescribing the
/// resolution of the whole surface. The brush can request finer detail.
pub(in crate::sculpt_session) fn input_spacing_mm(verts: &[f32], tris: &[u32]) -> f64 {
    let point = |v: u32| {
        let i = v as usize * 3;
        DVec3::new(verts[i] as f64, verts[i + 1] as f64, verts[i + 2] as f64)
    };
    let mut samples = Vec::with_capacity(tris.len() / 3);
    let mut area_sum = 0.0;
    for face in tris.as_chunks::<3>().0 {
        let p = [point(face[0]), point(face[1]), point(face[2])];
        let area = (p[1] - p[0]).cross(p[2] - p[0]).length();
        let mut edges = [
            (p[1] - p[0]).length(),
            (p[2] - p[1]).length(),
            (p[0] - p[2]).length(),
        ];
        edges.sort_by(f64::total_cmp);
        if area.is_finite() && area > 0.0 && edges[1].is_finite() && edges[1] > 0.0 {
            samples.push((edges[1], area));
            area_sum += area;
        }
    }
    samples.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
    let mut area = 0.0;
    for (length, weight) in samples {
        area += weight;
        if area >= area_sum * 0.5 {
            return length;
        }
    }
    f64::NAN
}

impl SculptSession {
    pub(crate) fn target_mm(&self, radius: f64, policy: &RemeshPolicy) -> Option<f64> {
        let requested = policy.target_for_radius(radius)?;
        Some(
            if self.input_spacing_mm.is_finite() && self.input_spacing_mm > 0.0 {
                self.input_spacing_mm.min(requested)
            } else {
                requested
            },
        )
    }
}
