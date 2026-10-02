//! Synthetic geometry operators with area measured from independent triangle sums.
//!
//! Half-space clipping is independently derived from Sutherland and Hodgman,
//! "Reentrant Polygon Clipping" (1974), <https://doi.org/10.1145/360767.360802>.
//! Intersections are welded by original edge identity, then angular windows are
//! bisected against measured area rather than retained vertex counts.
use super::arch::{self};
use super::{NoiseSpec, Rng, SyntheticMesh};
use glam::DVec3;
use occluview_align::Rigid;

#[derive(Clone, Copy)]
pub enum CropWindow {
    Centered,
    OneSided,
    TwoIslands,
}

/// Clip triangle polygons at angular thresholds, then retriangulate the boundary.
pub fn crop_by_area(mesh: &SyntheticMesh, fraction: f64, window: CropWindow) -> SyntheticMesh {
    let min = mesh
        .parameters
        .iter()
        .map(|uv| uv[0])
        .fold(f64::INFINITY, f64::min);
    let max = mesh
        .parameters
        .iter()
        .map(|uv| uv[0])
        .fold(f64::NEG_INFINITY, f64::max);
    let wanted = mesh.area() * fraction.clamp(0., 1.);
    let mut lo = 0f64;
    let mut hi = 1f64;
    for _ in 0..36 {
        let mid = lo.midpoint(hi);
        let crop = window_crop(mesh, min, max, mid, window);
        if crop.area() < wanted {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    window_crop(mesh, min, max, lo.midpoint(hi), window)
}
fn window_crop(
    mesh: &SyntheticMesh,
    min: f64,
    max: f64,
    ratio: f64,
    window: CropWindow,
) -> SyntheticMesh {
    match window {
        CropWindow::OneSided => clip(mesh, min, min + (max - min) * ratio),
        CropWindow::Centered => {
            let c = min.midpoint(max);
            clip(
                mesh,
                c - (max - min) * ratio * 0.5,
                c + (max - min) * ratio * 0.5,
            )
        }
        CropWindow::TwoIslands => {
            let mut a = clip(mesh, min, min + (max - min) * ratio * 0.5);
            let b = clip(mesh, max - (max - min) * ratio * 0.5, max);
            a.append(&b);
            a
        }
    }
}
#[derive(Clone, Copy)]
struct Corner {
    p: DVec3,
    uv: [f64; 2],
}
fn clip(mesh: &SyntheticMesh, min: f64, max: f64) -> SyntheticMesh {
    let mut out = SyntheticMesh {
        positions: Vec::new(),
        triangles: Vec::new(),
        region_ids: Vec::new(),
        parameters: Vec::new(),
        analytic_surface_id: mesh.analytic_surface_id,
        spec: mesh.spec.clone(),
    };
    let mut vertices = std::collections::BTreeMap::new();
    for (id, t) in mesh.triangles.as_chunks::<3>().0.iter().enumerate() {
        let mut polygon: Vec<_> = t
            .iter()
            .map(|&i| Corner {
                p: mesh.point(i as usize),
                uv: mesh.parameters[i as usize],
            })
            .collect();
        for (bound, lower) in [(min, true), (max, false)] {
            let mut next = Vec::new();
            for i in 0..polygon.len() {
                let a = polygon[i];
                let b = polygon[(i + 1) % polygon.len()];
                let inside = |c: Corner| {
                    if lower {
                        c.uv[0] >= bound
                    } else {
                        c.uv[0] <= bound
                    }
                };
                if inside(a) {
                    next.push(a);
                }
                if inside(a) != inside(b) {
                    // Canonical edge direction makes shared intersection floats identical.
                    let (a, b) = if a.uv[0] <= b.uv[0] { (a, b) } else { (b, a) };
                    let f = (bound - a.uv[0]) / (b.uv[0] - a.uv[0]);
                    next.push(Corner {
                        p: a.p + (b.p - a.p) * f,
                        uv: [bound, a.uv[1] + (b.uv[1] - a.uv[1]) * f],
                    });
                }
            }
            polygon = next;
        }
        for i in 1..polygon.len().saturating_sub(1) {
            let corners = [polygon[0], polygon[i], polygon[i + 1]];
            if (corners[1].p - corners[0].p)
                .cross(corners[2].p - corners[0].p)
                .length()
                <= 1e-12
            {
                continue;
            }
            for c in corners {
                let p = c.p.to_array().map(|x| x as f32);
                let key = p.map(f32::to_bits);
                let raw = *vertices.entry(key).or_insert_with(|| {
                    let index = (out.positions.len() / 3) as u32;
                    out.positions.extend(p);
                    out.parameters.push(c.uv);
                    index
                });
                out.triangles.push(raw);
            }
            out.region_ids.push(mesh.region_ids[id]);
        }
    }
    out
}

/// Transform moving geometry by inverse truth; truth maps it back to fixed.
pub fn rigid_offset(mesh: &SyntheticMesh, truth: Rigid) -> SyntheticMesh {
    let mut out = mesh.clone();
    let inverse = truth.inverse();
    for p in out.positions.as_chunks_mut::<3>().0.iter_mut() {
        let q = inverse.apply(DVec3::new(
            f64::from(p[0]),
            f64::from(p[1]),
            f64::from(p[2]),
        ));
        p.copy_from_slice(&q.to_array().map(|x| x as f32));
    }
    out
}
/// Independent analytic remesh with approximately the stated area density factor.
pub fn resample_density(mesh: &SyntheticMesh, factor: f64) -> SyntheticMesh {
    let mut spec = mesh.spec.clone().unwrap_or_default();
    if (factor - 1.).abs() < 1e-12 {
        spec.grid = [193, 47];
        spec.remesh_offset = 0.37;
        return arch::dental_arch(&spec);
    }
    let root = factor.sqrt();
    spec.grid = [
        ((spec.grid[0] as f64 * root).round() as usize).clamp(2, 300),
        ((spec.grid[1] as f64 * root).round() as usize).clamp(2, 60),
    ];
    spec.remesh_offset = 0.37;
    arch::dental_arch(&spec)
}

/// Independent clipped Gaussian normal displacement; correlated mode uses a
/// continuous trilinear 1 mm noise lattice, rather than per-vertex reuse.
pub fn normal_noise(mesh: &SyntheticMesh, noise: NoiseSpec) -> SyntheticMesh {
    let mut out = mesh.clone();
    let mut rng = Rng(noise.seed ^ mesh.positions.len() as u64);
    let mut region = vec![0u16; mesh.positions.len() / 3];
    for (t, &r) in mesh
        .triangles
        .as_chunks::<3>()
        .0
        .iter()
        .zip(&mesh.region_ids)
    {
        for &i in t {
            region[i as usize] = r;
        }
    }
    for (i, &region_id) in region.iter().enumerate() {
        let p = mesh.point(i);
        let n = mesh.spec.as_ref().map_or(DVec3::Z, |s| {
            arch::analytic_normal(s, mesh.parameters[i], region_id)
        });
        let value = noise.correlation_mm.map_or_else(
            || rng.gaussian(),
            |length| correlated(p, length, noise.seed),
        );
        let q = p + n * (value.clamp(-3., 3.) * noise.sigma_mm);
        out.positions[3 * i..3 * i + 3].copy_from_slice(&q.to_array().map(|x| x as f32));
    }
    out
}
fn correlated(p: DVec3, length: f64, seed: u64) -> f64 {
    let p = p / length;
    let base = p.floor();
    let f = p - base;
    let mut value = 0.;
    let mut norm = 0.;
    for x in 0..2 {
        for y in 0..2 {
            for z in 0..2 {
                let cell = base + DVec3::new(f64::from(x), f64::from(y), f64::from(z));
                let mut key = seed;
                for v in cell.to_array() {
                    key = key.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ v.to_bits();
                }
                let weight = if x == 0 { 1. - f.x } else { f.x }
                    * if y == 0 { 1. - f.y } else { f.y }
                    * if z == 0 { 1. - f.z } else { f.z };
                value += weight * Rng(key).gaussian();
                norm += weight * weight;
            }
        }
    }
    value / norm.sqrt()
}
pub fn invert_winding(mesh: &SyntheticMesh, mixed: bool) -> SyntheticMesh {
    let mut out = mesh.clone();
    for (i, t) in out.triangles.as_chunks_mut::<3>().0.iter_mut().enumerate() {
        if !mixed || i.is_multiple_of(4) {
            t.swap(1, 2);
        }
    }
    out
}
pub fn scale_geometry(mesh: &SyntheticMesh, factor: f64) -> SyntheticMesh {
    let mut out = mesh.clone();
    for x in &mut out.positions {
        *x = (f64::from(*x) * factor) as f32;
    }
    out
}

/// Exclusion mask from a deliberately selected reference region; labels do not
/// enter the solver in any other form.
pub fn reference_roi(mesh: &SyntheticMesh, regions: &[u16]) -> Vec<u8> {
    let mut mask = vec![1; mesh.positions.len() / 3];
    for (t, r) in mesh
        .triangles
        .as_chunks::<3>()
        .0
        .iter()
        .zip(&mesh.region_ids)
    {
        if regions.contains(r) {
            for &i in t {
                mask[i as usize] = 0;
            }
        }
    }
    mask
}
pub fn change_region(mesh: &SyntheticMesh, fraction: f64, lift_mm: f64) -> SyntheticMesh {
    let patch = crop_by_area(mesh, fraction, CropWindow::OneSided);
    let cutoff = patch
        .parameters
        .iter()
        .map(|uv| uv[0])
        .fold(f64::NEG_INFINITY, f64::max);
    let mut out = mesh.clone();
    for (i, uv) in mesh.parameters.iter().enumerate() {
        if uv[0] <= cutoff {
            out.positions[3 * i + 2] += lift_mm as f32;
        }
    }
    out
}
pub fn repeat_patch(mesh: &SyntheticMesh, copies: usize, pitch_mm: f64) -> SyntheticMesh {
    let mut out = mesh.clone();
    for i in 1..copies {
        let mut extra = mesh.clone();
        for p in extra.positions.as_chunks_mut::<3>().0.iter_mut() {
            p[0] += (i as f64 * pitch_mm) as f32;
        }
        out.append(&extra);
    }
    out
}
/// Replace the requested measured area by a disconnected unrelated patch.
pub fn outliers(mesh: &SyntheticMesh, fraction: f64, seed: u64) -> SyntheticMesh {
    let mut out = crop_by_area(mesh, 1. - fraction, CropWindow::Centered);
    let wanted = mesh.area() - out.area();
    let mut patch = arch::plane();
    let factor = (wanted / patch.area()).sqrt();
    patch = scale_geometry(&patch, factor);
    let mut rng = Rng(seed);
    let anchor = mesh.point(0) + DVec3::new(0., 0., 2. + 8. * rng.uniform());
    for p in patch.positions.as_chunks_mut::<3>().0.iter_mut() {
        p[0] += anchor.x as f32;
        p[1] += anchor.y as f32;
        p[2] += anchor.z as f32;
    }
    patch.region_ids.fill(5);
    out.append(&patch);
    out
}
/// Unrelated outer handle with measured area ten times the gum surface.
pub fn outer_extent(shell: &SyntheticMesh, gum_area: f64) -> SyntheticMesh {
    let mut out = shell.clone();
    let patch = arch::plane();
    let mut patch = scale_geometry(&patch, (10. * gum_area / patch.area()).sqrt());
    for p in patch.positions.as_chunks_mut::<3>().0.iter_mut() {
        p[1] -= 100.;
        p[2] += 10.;
    }
    patch.region_ids.fill(6);
    out.append(&patch);
    out
}

/// Identical analytic surfaces with an exact 1:20 triangle-density ratio,
/// without reusing geometry or nearest correspondences.
pub fn density_pair(spec: &arch::ArchSpec) -> [SyntheticMesh; 2] {
    let sparse = arch::ArchSpec {
        grid: [32, 8],
        remesh_offset: 0.,
        ..spec.clone()
    };
    let dense = arch::ArchSpec {
        grid: [128, 39],
        remesh_offset: 0.37,
        ..spec.clone()
    };
    [arch::dental_arch(&sparse), arch::dental_arch(&dense)]
}
/// Deliberately identical shell alternatives separated by two millimetres.
pub fn twin_surfaces(mesh: &SyntheticMesh) -> SyntheticMesh {
    let mut twin = mesh.clone();
    let mut outer = mesh.clone();
    for p in outer.positions.as_chunks_mut::<3>().0 {
        p[2] += 2.;
    }
    twin.append(&outer);
    twin
}
