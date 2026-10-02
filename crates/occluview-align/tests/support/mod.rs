//! Synthetic-only registration fixtures; truth never enters registration inputs.
//!
//! Gaussian displacements use an independently expressed polar-to-Cartesian
//! transform from Box and Muller (1958),
//! <https://doi.org/10.1214/aoms/1177706645>. Streams are fixed and separate for
//! independently sampled surfaces; no operating-system randomness is used.
#![allow(
    dead_code,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::float_cmp
)]
pub mod arch;
pub mod metrics;
pub mod operators;
use glam::DVec3;
use occluview_align::{Rigid, Soup};

#[derive(Clone, Debug)]
pub struct SyntheticMesh {
    pub positions: Vec<f32>,
    pub triangles: Vec<u32>,
    pub region_ids: Vec<u16>,
    pub analytic_surface_id: u64,
    pub parameters: Vec<[f64; 2]>,
    pub spec: Option<arch::ArchSpec>,
}
impl SyntheticMesh {
    pub fn soup(&self) -> Soup<'_> {
        Soup {
            positions: &self.positions,
            indices: &self.triangles,
            mask: None,
        }
    }
    pub fn point(&self, i: usize) -> DVec3 {
        DVec3::new(
            f64::from(self.positions[3 * i]),
            f64::from(self.positions[3 * i + 1]),
            f64::from(self.positions[3 * i + 2]),
        )
    }
    pub fn area(&self) -> f64 {
        self.triangles
            .as_chunks::<3>()
            .0
            .iter()
            .map(|t| {
                let a = self.point(t[0] as usize);
                let b = self.point(t[1] as usize);
                let c = self.point(t[2] as usize);
                (b - a).cross(c - a).length() * 0.5
            })
            .sum()
    }
    pub fn append(&mut self, other: &Self) {
        let offset = (self.positions.len() / 3) as u32;
        self.positions.extend_from_slice(&other.positions);
        self.triangles
            .extend(other.triangles.iter().map(|i| *i + offset));
        self.parameters.extend_from_slice(&other.parameters);
        self.region_ids.extend_from_slice(&other.region_ids);
    }
}
#[derive(Clone, Copy, Debug)]
pub struct NoiseSpec {
    pub sigma_mm: f64,
    pub seed: u64,
    pub correlation_mm: Option<f64>,
}
#[derive(Clone, Debug)]
pub struct CaseTruth {
    pub rigid: Rigid,
    pub common_regions: Vec<u16>,
    pub common_area: f64,
    pub symmetry_group: Vec<Rigid>,
    pub noise_spec: NoiseSpec,
}
/// Fixed family PRNG: `SplitMix64` integer mixer, independently expressed.
pub struct Rng(pub u64);
impl Rng {
    pub fn uniform(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        ((z ^ (z >> 31)) >> 11) as f64 / 9_007_199_254_740_992.
    }
    pub fn gaussian(&mut self) -> f64 {
        (-2. * self.uniform().max(f64::MIN_POSITIVE).ln()).sqrt()
            * (std::f64::consts::TAU * self.uniform()).cos()
    }
}
