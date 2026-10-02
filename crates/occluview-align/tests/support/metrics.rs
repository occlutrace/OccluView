//! Independent reference metrics. Plane information follows Gelfand et al.
//! (2003), <https://pixl.cs.princeton.edu/pubs/Gelfand_2003_GSS/stabicp.pdf>;
//! centered rotation columns are divided by physical RMS patch radius.
use glam::DVec3;
use occluview_align::{AlignmentSearchResult, Rigid};

pub fn pose_error(estimated: Rigid, truth: Rigid) -> [f64; 2] {
    [
        2. * estimated
            .rotation
            .dot(truth.rotation)
            .abs()
            .clamp(0., 1.)
            .acos()
            .to_degrees(),
        estimated.translation.distance(truth.translation),
    ]
}
pub fn probe_error(estimated: Rigid, truth: Rigid, probes: &[DVec3]) -> [f64; 2] {
    let mut squared = 0.;
    let mut max = 0f64;
    for &p in probes {
        let d = estimated.apply(p).distance(truth.apply(p));
        squared += d * d;
        max = max.max(d);
    }
    [(squared / probes.len().max(1) as f64).sqrt(), max]
}
pub fn top_k_recall(result: &AlignmentSearchResult, truth: Rigid, tolerance: [f64; 2]) -> bool {
    result.candidates.iter().any(|c| {
        let e = pose_error(c.pose, truth);
        e[0] <= tolerance[0] && e[1] <= tolerance[1]
    })
}
pub fn overlap_error(estimated: f64, truth: f64, smaller_area: f64) -> f64 {
    (estimated - truth).abs() / smaller_area
}
pub fn symmetry_error(
    estimated: Rigid,
    truth: Rigid,
    symmetries: &[Rigid],
    probes: &[DVec3],
) -> f64 {
    symmetries
        .iter()
        .map(|s| probe_error(estimated, truth.compose(s), probes)[0])
        .fold(probe_error(estimated, truth, probes)[0], f64::min)
}

/// Exact-derivative, deterministic analytic probes; never supplied to search.
pub fn analytic_probes(spec: &super::arch::ArchSpec) -> Vec<DVec3> {
    (0..1024)
        .map(|i| {
            let theta = spec.theta_range[0]
                + (spec.theta_range[1] - spec.theta_range[0]) * (f64::from(i) + 0.5) / 1024.;
            let v = -spec.gum_half_width_mm
                + 2. * spec.gum_half_width_mm * (f64::from(i * 73 % 1024) + 0.5) / 1024.;
            super::arch::band(spec, theta, v).0
        })
        .collect()
}

pub fn information_eigenvalues(pairs: &[(DVec3, DVec3)]) -> [f64; 6] {
    weighted_information(&pairs.iter().map(|&(p, n)| (p, n, 1.)).collect::<Vec<_>>())
}

#[expect(
    clippy::many_single_char_names,
    clippy::needless_range_loop,
    reason = "fixed Jacobi matrix row and pivot indices follow conventional algebra"
)]
pub fn weighted_information(pairs: &[(DVec3, DVec3, f64)]) -> [f64; 6] {
    let weight = pairs
        .iter()
        .map(|p| p.2)
        .sum::<f64>()
        .max(f64::MIN_POSITIVE);
    let center = pairs.iter().map(|p| p.0 * p.2).sum::<DVec3>() / weight;
    let radius = (pairs
        .iter()
        .map(|p| p.0.distance_squared(center) * p.2)
        .sum::<f64>()
        / weight)
        .sqrt()
        .max(1e-12);
    let mut h = [[0.; 6]; 6];
    for &(p, n, w) in pairs {
        let torque = (p - center).cross(n) / radius;
        let row = [torque.x, torque.y, torque.z, n.x, n.y, n.z];
        for i in 0..6 {
            for j in 0..6 {
                h[i][j] += row[i] * row[j] * w / weight;
            }
        }
    }
    // Fixed serial largest-offdiagonal Jacobi pivots with deterministic ties.
    for _ in 0..128 {
        let mut largest = 0.;
        let mut pivot = (0, 1);
        for (i, row) in h.iter().enumerate() {
            for (j, v) in row.iter().enumerate().skip(i + 1) {
                if v.abs() > largest {
                    largest = v.abs();
                    pivot = (i, j);
                }
            }
        }
        if largest < 1e-14 {
            break;
        }
        let (p, q) = pivot;
        let phi = 0.5 * (2. * h[p][q]).atan2(h[q][q] - h[p][p]);
        let (s, c) = phi.sin_cos();
        let pp = c * c * h[p][p] - 2. * s * c * h[p][q] + s * s * h[q][q];
        let qq = s * s * h[p][p] + 2. * s * c * h[p][q] + c * c * h[q][q];
        for i in 0..6 {
            if i != p && i != q {
                let a = h[i][p];
                let b = h[i][q];
                h[i][p] = c * a - s * b;
                h[p][i] = h[i][p];
                h[i][q] = s * a + c * b;
                h[q][i] = h[i][q];
            }
        }
        h[p][p] = pp;
        h[q][q] = qq;
        h[p][q] = 0.;
        h[q][p] = 0.;
    }
    let mut values = std::array::from_fn(|i| h[i][i]);
    values.sort_by(f64::total_cmp);
    values
}

/// Independent known common-region information and occupied 1 mm cells.
pub fn reference_information(mesh: &super::SyntheticMesh) -> ([f64; 6], usize) {
    let spec = mesh.spec.as_ref().unwrap();
    let mut cells = std::collections::BTreeSet::new();
    let pairs: Vec<_> = mesh
        .triangles
        .as_chunks::<3>()
        .0
        .iter()
        .zip(&mesh.region_ids)
        .map(|(t, &region)| {
            let points = t.map(|i| mesh.point(i as usize));
            let area = (points[1] - points[0])
                .cross(points[2] - points[0])
                .length()
                * 0.5;
            let uv = t.map(|i| {
                let mut uv = mesh.parameters[i as usize];
                if region == 1 && uv[1] < 0. {
                    uv[1] = 1.;
                }
                uv
            });
            let uv = [
                (uv[0][0] + uv[1][0] + uv[2][0]) / 3.,
                (uv[0][1] + uv[1][1] + uv[2][1]) / 3.,
            ];
            let (p, dt, dv) = if region == 0 {
                super::arch::band(spec, uv[0], uv[1])
            } else {
                super::arch::palate(spec, uv[0], uv[1])
            };
            let n = dt.cross(dv).normalize_or_zero();
            cells.insert(p.floor().to_array().map(|x| x as i64));
            (p, n, area)
        })
        .collect();
    (weighted_information(&pairs), cells.len())
}
