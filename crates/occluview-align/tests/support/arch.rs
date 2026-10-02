//! Analytic dental-like surfaces, not an anatomical model or clinical corpus.
use super::{Rng, SyntheticMesh};
use glam::DVec3;
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct ArchSpec {
    pub a_mm: f64,
    pub b_mm: f64,
    pub theta_range: [f64; 2],
    pub gum_half_width_mm: f64,
    pub crest_mm: f64,
    pub palate_mm: f64,
    pub teeth: usize,
    pub asymmetry_seed: u64,
    pub grid: [usize; 2],
    pub winding: bool,
    /// Interior parameter jitter for independently remeshed scans.
    pub remesh_offset: f64,
}
impl Default for ArchSpec {
    fn default() -> Self {
        Self {
            a_mm: 25.,
            b_mm: 31.,
            theta_range: [-2.1, 2.1],
            gum_half_width_mm: 4.,
            crest_mm: 2.,
            palate_mm: 3.,
            teeth: 14,
            asymmetry_seed: 1,
            grid: [160, 40],
            winding: false,
            remesh_offset: 0.,
        }
    }
}

/// Position and exact analytic tangent derivatives on the gum/tooth band.
pub fn band(spec: &ArchSpec, theta: f64, v: f64) -> (DVec3, DVec3, DVec3) {
    let phase = 0.2;
    let mut z = spec.crest_mm * (0.85 * theta).cos() - 0.1 * v * v
        + 0.65 * (2.3 * theta + phase).sin()
        + 0.08 * v * (1.7 * theta).cos();
    let mut zt = -0.85 * spec.crest_mm * (0.85 * theta).sin()
        + 0.65 * 2.3 * (2.3 * theta + phase).cos()
        - 0.08 * 1.7 * v * (1.7 * theta).sin();
    let mut zv = -0.2 * v + 0.08 * (1.7 * theta).cos();
    let mut rng = Rng(spec.asymmetry_seed);
    for i in 0..spec.teeth {
        let center = -1.75
            + 3.5 * i as f64 / (spec.teeth.saturating_sub(1).max(1)) as f64
            + (rng.uniform() * 2. - 1.) * 0.03;
        let h = 3.5 + 1.5 * rng.uniform();
        let dt = theta - center;
        let e = h * (-(dt / 0.09).powi(4) - (v / 1.4).powi(4)).exp();
        let modulate = 1. + 0.15 * (3. * v).cos() + 0.12 * (28. * dt).sin();
        z += e * modulate;
        zt += e * (-4. * dt.powi(3) / 0.09f64.powi(4) * modulate + 0.12 * 28. * (28. * dt).cos());
        zv += e * (-4. * v.powi(3) / 1.4f64.powi(4) * modulate - 0.15 * 3. * (3. * v).sin());
    }
    (
        DVec3::new(
            (spec.a_mm + v) * theta.sin(),
            (spec.b_mm + v) * theta.cos(),
            z,
        ),
        DVec3::new(
            (spec.a_mm + v) * theta.cos(),
            -(spec.b_mm + v) * theta.sin(),
            zt,
        ),
        DVec3::new(theta.sin(), theta.cos(), zv),
    )
}

/// Elliptical interior dome, sharing the inner band rim exactly.
pub fn palate(spec: &ArchSpec, theta: f64, r: f64) -> (DVec3, DVec3, DVec3) {
    let (rim, dt, _) = band(spec, theta, -spec.gum_half_width_mm);
    let z = r * rim.z - spec.palate_mm * (1. - r * r) + 0.6 * r * (1. - r) * (3. * theta).sin();
    (
        DVec3::new(r * rim.x, r * rim.y, z),
        DVec3::new(
            r * dt.x,
            r * dt.y,
            r * dt.z + 1.8 * r * (1. - r) * (3. * theta).cos(),
        ),
        DVec3::new(
            rim.x,
            rim.y,
            rim.z + 2. * spec.palate_mm * r + 0.6 * (1. - 2. * r) * (3. * theta).sin(),
        ),
    )
}

pub fn analytic_normal(spec: &ArchSpec, uv: [f64; 2], region: u16) -> DVec3 {
    let mut uv = uv;
    if region == 1 && uv[1] < 0. {
        uv[1] = 1.;
    }
    let (_, dt, dv) = if region == 0 {
        band(spec, uv[0], uv[1])
    } else {
        palate(spec, uv[0], uv[1])
    };
    let n = dt.cross(dv).normalize_or_zero();
    let n = if region == 1 { -n } else { n };
    if spec.winding {
        -n
    } else {
        n
    }
}

pub fn dental_arch(spec: &ArchSpec) -> SyntheticMesh {
    let [nt, nv] = spec.grid;
    let nt = nt.clamp(2, 300);
    let nv = nv.clamp(2, 60);
    let mut mesh = SyntheticMesh {
        positions: Vec::new(),
        triangles: Vec::new(),
        region_ids: Vec::new(),
        analytic_surface_id: spec.asymmetry_seed,
        parameters: Vec::new(),
        spec: Some(spec.clone()),
    };
    for i in 0..=nt {
        let t = spec.theta_range[0]
            + (spec.theta_range[1] - spec.theta_range[0])
                * grid_fraction(i, nt, spec.remesh_offset);
        for j in 0..=nv {
            let v = -spec.gum_half_width_mm
                + 2. * spec.gum_half_width_mm * grid_fraction(j, nv, spec.remesh_offset);
            mesh.positions
                .extend(band(spec, t, v).0.to_array().map(|x| x as f32));
            mesh.parameters.push([t, v]);
        }
    }
    for i in 0..nt {
        for j in 0..nv {
            let a = (i * (nv + 1) + j) as u32;
            let b = a + (nv + 1) as u32;
            triangle(&mut mesh, [a, b, a + 1], 0, spec.winding);
            triangle(&mut mesh, [a + 1, b, b + 1], 0, spec.winding);
        }
    }
    if spec.palate_mm > 0. {
        let center = (mesh.positions.len() / 3) as u32;
        mesh.positions.extend([0., 0., -spec.palate_mm as f32]);
        mesh.parameters.push([0., 0.]);
        let mut previous = vec![center; nt + 1];
        for j in 1..=nv {
            let r = grid_fraction(j, nv, spec.remesh_offset);
            let mut current = Vec::new();
            for i in 0..=nt {
                let t = spec.theta_range[0]
                    + (spec.theta_range[1] - spec.theta_range[0])
                        * grid_fraction(i, nt, spec.remesh_offset);
                if j == nv {
                    current.push((i * (nv + 1)) as u32);
                } else {
                    current.push((mesh.positions.len() / 3) as u32);
                    mesh.positions
                        .extend(palate(spec, t, r).0.to_array().map(|x| x as f32));
                    mesh.parameters.push([t, r]);
                }
            }
            for i in 0..nt {
                triangle(
                    &mut mesh,
                    [previous[i], current[i], current[i + 1]],
                    1,
                    !spec.winding,
                );
                if j > 1 {
                    triangle(
                        &mut mesh,
                        [previous[i], current[i + 1], previous[i + 1]],
                        1,
                        !spec.winding,
                    );
                }
            }
            previous = current;
        }
    }
    mesh
}
fn triangle(mesh: &mut SyntheticMesh, mut t: [u32; 3], region: u16, invert: bool) {
    if invert {
        t.swap(1, 2);
    }
    mesh.triangles.extend(t);
    mesh.region_ids.push(region);
}

/// Closed synthetic prosthesis, with reversed intaglio and different outer cusps.
pub fn prosthesis_shell(
    gum: &SyntheticMesh,
    thickness_mm: f64,
    outer_spec: &ArchSpec,
) -> SyntheticMesh {
    let mut shell = gum.clone();
    for t in shell.triangles.as_chunks_mut::<3>().0.iter_mut() {
        t.swap(1, 2);
    }
    shell.region_ids.fill(2);
    let mut outer = dental_arch(outer_spec);
    // Matching topology is required to stitch every boundary, with shared rims.
    assert_eq!(outer.positions.len(), gum.positions.len());
    assert_eq!(outer.triangles.len(), gum.triangles.len());
    for p in outer.positions.as_chunks_mut::<3>().0.iter_mut() {
        p[2] += thickness_mm as f32;
    }
    outer.region_ids.fill(3);
    let offset = (gum.positions.len() / 3) as u32;

    let mut edges = BTreeMap::new();
    for t in gum.triangles.as_chunks::<3>().0 {
        for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            let e = if a < b { (a, b) } else { (b, a) };
            let entry = edges.entry(e).or_insert((0usize, (a, b)));
            entry.0 += 1;
        }
    }
    shell.append(&outer);
    for (_, (count, (a, b))) in edges {
        if count == 1 {
            triangle(&mut shell, [a, b, b + offset], 4, false);
            triangle(&mut shell, [a, b + offset, a + offset], 4, false);
        }
    }
    shell
}

pub fn plane() -> SyntheticMesh {
    primitive(0)
}
pub fn cylinder() -> SyntheticMesh {
    primitive(1)
}
pub fn sphere() -> SyntheticMesh {
    primitive(2)
}
fn primitive(kind: u8) -> SyntheticMesh {
    let mut mesh = SyntheticMesh {
        positions: Vec::new(),
        triangles: Vec::new(),
        region_ids: Vec::new(),
        parameters: Vec::new(),
        analytic_surface_id: 100 + u64::from(kind),
        spec: None,
    };
    for i in 0..=40 {
        for j in 0..=20 {
            let u = f64::from(i) / 40.;
            let v = f64::from(j) / 20.;
            let a = std::f64::consts::TAU * u;
            let p = match kind {
                0 => DVec3::new(40. * u - 20., 40. * v - 20., 0.),
                1 => DVec3::new(10. * a.cos(), 10. * a.sin(), 30. * v - 15.),
                _ => {
                    let b = std::f64::consts::PI * v;
                    DVec3::new(
                        15. * a.cos() * b.sin(),
                        15. * a.sin() * b.sin(),
                        15. * b.cos(),
                    )
                }
            };
            mesh.positions.extend(p.to_array().map(|x| x as f32));
            mesh.parameters.push([u, v]);
        }
    }
    for i in 0..40 {
        for j in 0..20 {
            let a = (i * 21 + j) as u32;
            triangle(&mut mesh, [a, a + 21, a + 1], 0, false);
            triangle(&mut mesh, [a + 1, a + 21, a + 22], 0, false);
        }
    }
    mesh
}
/// Frozen independently different width/depth and cusp layout, without solver feedback.
pub fn negative_arch_pair(seed: u64) -> (SyntheticMesh, SyntheticMesh) {
    let a = ArchSpec {
        asymmetry_seed: seed,
        grid: [40, 10],
        ..ArchSpec::default()
    };
    let b = ArchSpec {
        a_mm: a.a_mm * 1.2,
        b_mm: a.b_mm * 1.2,
        asymmetry_seed: seed.wrapping_add(1000),
        teeth: 11,
        ..a.clone()
    };
    (dental_arch(&a), dental_arch(&b))
}

fn grid_fraction(index: usize, cells: usize, offset: f64) -> f64 {
    let u = index as f64 / cells as f64;
    u + offset * (std::f64::consts::PI * u).sin() * (1.7 * index as f64).sin() / cells as f64
}

/// Exactly two million triangles; only budget tests construct this stress mesh.
pub fn dense_stress_plane() -> SyntheticMesh {
    let mut mesh = SyntheticMesh {
        positions: Vec::with_capacity(3 * 1001 * 1001),
        triangles: Vec::with_capacity(6_000_000),
        region_ids: vec![0; 2_000_000],
        parameters: Vec::with_capacity(1001 * 1001),
        analytic_surface_id: 999,
        spec: None,
    };
    for x in 0..=1000 {
        for y in 0..=1000 {
            mesh.positions
                .extend([x as f32 * 0.04, y as f32 * 0.04, 0.]);
            mesh.parameters
                .push([f64::from(x) / 1000., f64::from(y) / 1000.]);
        }
    }
    for x in 0..1000 {
        for y in 0..1000 {
            let a = (x * 1001 + y) as u32;
            mesh.triangles
                .extend([a, a + 1001, a + 1, a + 1, a + 1001, a + 1002]);
        }
    }
    mesh
}
