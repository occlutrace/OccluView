//! Independently derived area sampling and conditioned surface snapshots.
//!
//! Osada et al., Shape Distributions, ACM TOG 2002,
//! <https://gfx.cs.princeton.edu/pubs/Osada_2002_SD/tog02.pdf>, motivates
//! triangle-area selection rather than vertex-density weighting. Here the area
//! coordinate is stratified in source order with fixed integer seed streams.
//! For barycentric radius r, the triangle Jacobian is proportional to r, so
//! r=sqrt(u) gives uniform area. No reference implementation is used.
//! Spatial cell ownership separates training from holdout; cell hashing is an
//! engineering sampling policy, not a claim of independent scanner noise.

use super::vertex_at;
use crate::{
    AlignmentInputError, Completion, InputCheck, InputField, MeshInput, Metric, RegionPolicy,
    Rigid, SurfaceIndex,
};
use glam::{DAffine3, DVec3};
use occluview_geometry::surface::{BuildOutcome, GeometryControl, GeometryMemory, GeometryStop};

/// Fixed base seed; streams are mixed independently of input revisions and time.
pub const AREA_SAMPLE_SEED: u64 = 0x4f56_5f41_4c52_3100;
/// Coarse surface sample allowance per side.
pub const COARSE_AREA_SAMPLES: usize = 1_024;
/// Middle surface sample allowance per side.
pub const MID_AREA_SAMPLES: usize = 4_096;
/// Dense surface sample allowance per side.
pub const DENSE_AREA_SAMPLES: usize = 16_384;
/// Disjoint verification sample allowance per side.
pub const VERIFY_AREA_SAMPLES: usize = 8_192;

/// Side used to name numeric validation errors.
#[derive(Clone, Copy, Debug)]
pub enum SurfaceSide {
    /// Moving mesh.
    Moving,
    /// Fixed mesh.
    Fixed,
}
impl SurfaceSide {
    fn fields(self) -> (InputField, InputField) {
        match self {
            Self::Moving => (InputField::MovingPositions, InputField::MovingAffine),
            Self::Fixed => (InputField::FixedPositions, InputField::FixedAffine),
        }
    }
}

/// A f64 area representative; no vertex-density or known correspondence id.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceSample {
    /// Stable ordinal within the seed stream.
    pub id: u32,
    /// Point in the private centred frame, in millimetres.
    pub point: DVec3,
    /// Unit geometric normal, absent on incoherent/singular geometry.
    pub normal: Option<DVec3>,
    /// Source triangle ordinal.
    pub triangle: u32,
    /// Weights of the source triangle's three corners.
    pub barycentric: [f64; 3],
    /// Sampled eligible-area estimator in square millimetres.
    pub area_weight_mm2: f64,
    /// Operator region id; zero denotes the single included region.
    pub region_id: u16,
}

/// Population represented by a batch; roles occupy disjoint spatial cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SampleRole {
    /// Whole eligible surface before splitting.
    Full,
    /// Cells used for proposals and optimization.
    Training,
    /// Separate cells used for independent evidence.
    Holdout,
}

/// Bounded samples and their explicit area population and completeness.
#[derive(Debug)]
pub struct SampleBatch {
    /// Stable sample order; an interrupted batch is a partial prefix.
    pub samples: Vec<SurfaceSample>,
    /// Measured area of this declared population, in square millimetres.
    pub population_area_mm2: f64,
    /// Area weight actually represented by returned points.
    pub represented_area_mm2: f64,
    /// Eligible area outside the declared role; whole support contribution is
    /// bounded by zero and this area, rather than extrapolated from the role.
    pub unqueried_area_mm2: f64,
    /// Population role.
    pub role: SampleRole,
    /// Complete or an explicit interruption reason.
    pub completion: Completion,
    _memory: GeometryMemory,
}

/// Training/holdout populations occupy disjoint 1 mm cells.
#[derive(Debug)]
pub struct SampleSplit {
    /// Training cells.
    pub training: SampleBatch,
    /// Holdout cells, subdivided by [`spatial_stratum`].
    pub holdout: SampleBatch,
}

/// Reversible conditioning without physical scale changes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceFrame {
    /// Eligible area centroid in authored world coordinates.
    pub center_world: DVec3,
    /// Original local coordinates to centred f64 query coordinates.
    pub query_from_local: DAffine3,
}
impl SurfaceFrame {
    /// Convert a world correction to the two private centred frames.
    /// Returns `None` on internal finite-arithmetic overflow.
    pub fn correction_to_query(self, fixed: Self, pose: Rigid) -> Option<Rigid> {
        let translation = pose.apply(self.center_world) - fixed.center_world;
        translation.is_finite().then_some(Rigid {
            rotation: pose.rotation,
            translation,
        })
    }
    /// Restore a correction from the two centred frames to the snapshot world frame.
    /// Returns `None` on internal finite-arithmetic overflow.
    pub fn correction_to_world(self, fixed: Self, pose: Rigid) -> Option<Rigid> {
        let translation = fixed.center_world + pose.translation - pose.rotation * self.center_world;
        translation.is_finite().then_some(Rigid {
            rotation: pose.rotation,
            translation,
        })
    }
}

/// Explicit topology and orientation omissions; no missing residual becomes zero.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SurfaceQuality {
    /// Out-of-range connectivity records omitted.
    pub invalid_triangles: usize,
    /// Mask-straddling triangles omitted.
    pub excluded_triangles: usize,
    /// Triangles with double area at most 1e-12 mm squared omitted.
    pub degenerate_triangles: usize,
    /// Trailing position scalars, checked numerically but not usable as vertices.
    pub trailing_positions: usize,
    /// Trailing index entries ignored as incomplete topology.
    pub trailing_indices: usize,
    /// Completely accounted degenerate area, or unavailable on interruption.
    pub omitted_area_mm2: Metric<f64>,
    /// Directed adjacency and finite authored frame support signed normals.
    pub orientation_coherent: bool,
}

/// One immutable eligible surface and all declared independent sample streams.
#[derive(Debug)]
pub struct PreparedSurface {
    /// Coarse/middle/dense training and verification representatives.
    pub samples: [SampleBatch; 4],
    /// Declared eligible triangle representation in the centred frame.
    /// Original-surface authority exists only when `exact_original` is true.
    pub original_index: SurfaceIndex,
    /// True for the complete original surface; false for a bounded proxy.
    pub exact_original: bool,
    /// Measured area of the representation, distinct from original eligible area.
    pub represented_area_mm2: f64,
    /// Total eligible nondegenerate area; physical units are unchanged.
    pub eligible_area_mm2: f64,
    /// Meaning of the included population.
    pub regions: RegionPolicy,
    /// Topology and orientation limitations.
    pub quality: SurfaceQuality,
    /// Reversible private frame.
    pub frame: SurfaceFrame,
    /// Caller revision of the immutable geometry/pose/mask snapshot.
    pub revision: u64,
}

/// Surface availability and numeric/area accounting are independent.
#[derive(Debug)]
pub struct SurfacePreparation {
    /// Complete exact representation when it fits the advertised resources.
    pub surface: Option<PreparedSurface>,
    /// Completion reason, including empty geometry and resource refusal.
    pub completion: Completion,
    /// Numeric input coverage, including excluded/trailing positions.
    pub input_check: InputCheck,
    /// Full eligible-area accounting, even if index construction later stopped.
    pub eligible_area_mm2: Metric<f64>,
    /// Omitted topology and orientation evidence.
    pub quality: SurfaceQuality,
}

/// Prepare immutable f64 geometry, area accounting, exact index and disjoint samples.
///
/// Validation checks all scalars, including excluded vertices, in blocks of
/// 128. Source record limits, allocation admission and every loop share the
/// same deadline/cancellation. Oversized inputs are explicitly partial; no
/// arbitrary prefix claims full area. Finite arithmetic overflow is a resource
/// outcome. Singular finite authored frames never fabricate normal evidence.
///
/// # Errors
/// Returns the encountered input field/scalar for NaN or infinity only.
#[expect(
    clippy::too_many_lines,
    reason = "ordered numeric and area accounting before any exact representation claim"
)]
pub fn prepare_alignment_surface(
    mesh: MeshInput<'_>,
    side: SurfaceSide,
    regions: RegionPolicy,
    control: &GeometryControl,
) -> Result<SurfacePreparation, AlignmentInputError> {
    let mut result = SurfacePreparation {
        surface: None,
        completion: Completion::Complete,
        input_check: InputCheck::Partial {
            checked: 0,
            total: mesh.soup.positions.len().saturating_add(12),
        },
        eligible_area_mm2: Metric::default(),
        quality: SurfaceQuality {
            trailing_positions: mesh.soup.positions.len() % 3,
            trailing_indices: mesh.soup.indices.len() % 3,
            ..SurfaceQuality::default()
        },
    };
    let (position_field, affine_field) = side.fields();
    let mut checked = 0;
    for (i, scalar) in mesh
        .world_from_local
        .to_cols_array()
        .into_iter()
        .enumerate()
    {
        if let Err(stop) = control.charge_operations(1) {
            finish_partial(&mut result, checked, stop);
            return Ok(result);
        }
        if !scalar.is_finite() {
            return Err(AlignmentInputError::NonFinite {
                field: affine_field,
                index: i,
            });
        }
        checked += 1;
    }
    let scalar_limit = control
        .limits()
        .input_vertices
        .saturating_mul(3)
        .saturating_add(
            if mesh.soup.vertex_count() <= control.limits().input_vertices {
                mesh.soup.positions.len() % 3
            } else {
                0
            },
        );
    let scanned = mesh.soup.positions.len().min(scalar_limit);
    for (block_id, block) in mesh.soup.positions[..scanned].chunks(128).enumerate() {
        if let Err(stop) = control.charge_operations(block.len() as u64) {
            finish_partial(&mut result, checked, stop);
            return Ok(result);
        }
        for (offset, &scalar) in block.iter().enumerate() {
            if !scalar.is_finite() {
                return Err(AlignmentInputError::NonFinite {
                    field: position_field,
                    index: block_id * 128 + offset,
                });
            }
            checked += 1;
        }
    }
    if scanned != mesh.soup.positions.len() {
        finish_partial(&mut result, checked, GeometryStop::ResourceLimit);
        return Ok(result);
    }
    result.input_check = InputCheck::Complete;
    if mesh.soup.vertex_count() > control.limits().input_vertices
        || mesh.soup.triangle_count() > control.limits().input_triangles
    {
        result.completion = Completion::ResourceLimit;
        return Ok(result);
    }
    let mut area = 0.;
    let mut omitted = 0.;
    let mut center = DVec3::ZERO;
    for ids in mesh.soup.indices.as_chunks::<3>().0 {
        if let Err(stop) = control.charge_operations(1) {
            result.completion = completion(stop);
            return Ok(result);
        }
        if ids
            .iter()
            .any(|&id| (id as usize) >= mesh.soup.vertex_count())
        {
            result.quality.invalid_triangles += 1;
            continue;
        }
        if crate::mask::eligible_region(mesh.soup, *ids, regions).is_none() {
            result.quality.excluded_triangles += 1;
            continue;
        }
        let points = ids.map(|id| {
            vertex_at(mesh.soup.positions, id as usize)
                .map(|p| mesh.world_from_local.transform_point3(p))
        });
        let [Some(a), Some(b), Some(c)] = points else {
            result.completion = Completion::ResourceLimit;
            return Ok(result);
        };
        let double_area = (b - a).cross(c - a).length();
        if ![a, b, c].iter().all(|p| p.is_finite()) || !double_area.is_finite() {
            result.completion = Completion::ResourceLimit;
            return Ok(result);
        }
        if double_area <= 1e-12 {
            result.quality.degenerate_triangles += 1;
            omitted += double_area * 0.5;
            continue;
        }
        let next = area + double_area * 0.5;
        let centroid = a / 3. + b / 3. + c / 3.;
        center += (centroid - center) * (double_area * 0.5 / next);
        if !next.is_finite() || !center.is_finite() {
            result.completion = Completion::ResourceLimit;
            return Ok(result);
        }
        area = next;
    }
    result.eligible_area_mm2 = Metric::Measured(area);
    result.quality.omitted_area_mm2 = Metric::Measured(omitted);
    if area <= 0. {
        result.completion = Completion::NoUsableSurface;
        return Ok(result);
    }
    let frame = SurfaceFrame {
        center_world: center,
        query_from_local: DAffine3 {
            matrix3: mesh.world_from_local.matrix3,
            translation: mesh.world_from_local.translation - center,
        },
    };
    // Decide before allocation admission: an optional exact attempt must not
    // latch a global resource stop and prevent the bounded fallback. Leave
    // working room for the other surface, samples and connected patches.
    let estimate = mesh
        .soup
        .triangle_count()
        .saturating_mul(1024)
        .saturating_add(mesh.soup.vertex_count().saturating_mul(256))
        .saturating_add(4096);
    let available = control
        .limits()
        .memory_bytes
        .saturating_sub(usize::try_from(control.counters().memory_bytes).unwrap_or(usize::MAX));
    let exact_original = estimate <= available / 3;
    let mut index = if exact_original {
        match SurfaceIndex::build_controlled(mesh.soup, frame.query_from_local, control) {
            BuildOutcome::Complete(index) => index,
            BuildOutcome::Empty => {
                result.completion = Completion::NoUsableSurface;
                return Ok(result);
            }
            BuildOutcome::Partial { reason, .. } => {
                result.completion = completion(reason);
                return Ok(result);
            }
        }
    } else {
        match super::proxy::prepare_bounded_proxy(mesh, frame, regions, control) {
            Ok(index) => index,
            Err(stop) => {
                result.completion = completion(stop);
                return Ok(result);
            }
        }
    };
    let represented_area_mm2 = index.surface_area_mm2();
    result.quality.orientation_coherent = exact_original
        && index.orientation_coherent()
        && mesh.world_from_local.matrix3.determinant().is_finite()
        && mesh.world_from_local.matrix3.determinant() != 0.;
    index.set_query_control(control.clone());
    let population = match super::population::CellPopulation::build(
        &index,
        result.quality.orientation_coherent,
        control,
    ) {
        Ok(population) => population,
        Err(stop) => {
            result.completion = completion(stop);
            return Ok(result);
        }
    };
    let mut batches = Vec::new();
    if batches.try_reserve_exact(4).is_err() {
        result.completion = Completion::ResourceLimit;
        return Ok(result);
    }
    for (stream, budget) in [
        COARSE_AREA_SAMPLES,
        MID_AREA_SAMPLES,
        DENSE_AREA_SAMPLES,
        VERIFY_AREA_SAMPLES,
    ]
    .into_iter()
    .enumerate()
    {
        let role = if stream == 3 {
            SampleRole::Holdout
        } else {
            SampleRole::Training
        };
        let batch = match population.samples(
            role,
            budget,
            mix_seed(AREA_SAMPLE_SEED ^ stream as u64),
            control,
        ) {
            Ok(batch) => batch,
            Err(stop) => {
                result.completion = completion(stop);
                return Ok(result);
            }
        };
        if batch.completion != Completion::Complete {
            result.completion = batch.completion;
            return Ok(result);
        }
        batches.push(batch);
    }
    drop(population);
    let Ok(samples) = batches.try_into() else {
        result.completion = Completion::ResourceLimit;
        return Ok(result);
    };
    result.surface = Some(PreparedSurface {
        samples,
        original_index: index,
        exact_original,
        represented_area_mm2,
        eligible_area_mm2: area,
        regions,
        quality: result.quality.clone(),
        frame,
        revision: mesh.revision,
    });
    if !exact_original {
        result.completion = Completion::ResourceLimit;
    }
    Ok(result)
}

fn finish_partial(result: &mut SurfacePreparation, checked: usize, stop: GeometryStop) {
    if let InputCheck::Partial { total, .. } = result.input_check {
        result.input_check = InputCheck::Partial { checked, total };
    }
    result.completion = completion(stop);
}
/// Convert a neutral interruption to the registration completion contract.
pub fn completion(stop: GeometryStop) -> Completion {
    match stop {
        GeometryStop::Cancelled => Completion::Cancelled,
        GeometryStop::Deadline => Completion::Deadline,
        GeometryStop::WorkLimit => Completion::WorkLimit,
        GeometryStop::ResourceLimit | GeometryStop::Numerical => Completion::ResourceLimit,
    }
}

/// Deterministic area-stratified points, bounded independently of mesh density.
///
/// CDF accumulation follows original triangle order. Every stratum has equal
/// area weight; square-root barycentric jitter is strictly inside its facet.
/// Interrupted output is a marked partial prefix, never a completed population.
///
/// # Errors
/// Returns resource/work/deadline/cancellation exhaustion before unsafe growth,
/// or numerical failure when an area CDF cannot be represented.
#[allow(clippy::cast_precision_loss)]
pub fn area_samples(
    index: &SurfaceIndex,
    budget: usize,
    seed: u64,
    normals_reliable: bool,
    control: &GeometryControl,
) -> Result<SampleBatch, GeometryStop> {
    let count = index.triangle_count();
    let bytes = count
        .checked_mul(size_of::<(f64, u32, [DVec3; 3], DVec3)>())
        .ok_or(GeometryStop::ResourceLimit)?;
    let _cdf_memory = control.reserve(bytes)?;
    let mut cdf = Vec::new();
    cdf.try_reserve_exact(count)
        .map_err(|_| GeometryStop::ResourceLimit)?;
    let mut area = 0.;
    for (triangle, corners, normal) in index.triangles() {
        control.charge_operations(1)?;
        area += (corners[1] - corners[0])
            .cross(corners[2] - corners[0])
            .length()
            * 0.5;
        if !area.is_finite() {
            return Err(GeometryStop::Numerical);
        }
        cdf.push((area, triangle, corners, normal));
    }
    let mut result = empty_batch(budget, control)?;
    result.population_area_mm2 = area;
    if budget == 0 || area <= 0. {
        return Ok(result);
    }
    let weight = area / budget as f64;
    let mut state = seed;
    let mut slot = 0;
    for id in 0..budget {
        if let Err(stop) = control.charge_operations(1) {
            result.completion = completion(stop);
            break;
        }
        let coordinate = ((id as f64 + uniform(&mut state)) / budget as f64) * area;
        while cdf.get(slot).is_some_and(|entry| entry.0 <= coordinate) && slot + 1 < cdf.len() {
            control.charge_operations(1)?;
            slot += 1;
        }
        let Some(&(_, triangle, points, normal)) = cdf.get(slot) else {
            return Err(GeometryStop::Numerical);
        };
        let radial = uniform(&mut state).sqrt();
        let angular = uniform(&mut state);
        let barycentric = [1. - radial, radial * (1. - angular), radial * angular];
        let point =
            points[0] * barycentric[0] + points[1] * barycentric[1] + points[2] * barycentric[2];
        if !point.is_finite() {
            return Err(GeometryStop::Numerical);
        }
        result.samples.push(SurfaceSample {
            id: u32::try_from(id).map_err(|_| GeometryStop::ResourceLimit)?,
            point,
            normal: normals_reliable.then_some(normal),
            triangle,
            barycentric,
            area_weight_mm2: weight,
            region_id: 0,
        });
    }
    result.represented_area_mm2 = weight * result.samples.len() as f64;
    Ok(result)
}

/// Split by deterministic 1 mm cells, retaining at most `budget` per role.
///
/// Cells own a role, so adjacent points inside one cell cannot occur in both
/// populations. Eight independent low hash bits label holdout strata. Equal
/// area contributions define the input quadrature, not an estimate of twice
/// that population. Thinning transfers each omitted block's actual weight to
/// its selected representative. Prepared batches use exact clipped cell areas.
/// A deficient single-cell surface may have no
/// holdout, which later verification must report as missing evidence.
///
/// # Errors
/// Returns bounded allocation/work interruption or unrepresentable cell coordinates.
#[allow(clippy::cast_precision_loss)]
pub fn split_samples(
    samples: &[SurfaceSample],
    budget: usize,
    control: &GeometryControl,
) -> Result<SampleSplit, GeometryStop> {
    let mut counts = [0usize; 2];
    let mut areas = [0.; 2];
    for sample in samples {
        control.charge_operations(1)?;
        let role = usize::from(cell_hash(sample.point)? & 8 != 0);
        counts[role] += 1;
        if !sample.area_weight_mm2.is_finite() || sample.area_weight_mm2 < 0. {
            return Err(GeometryStop::Numerical);
        }
        areas[role] += sample.area_weight_mm2;
        if !areas[role].is_finite() {
            return Err(GeometryStop::Numerical);
        }
    }
    let mut training = empty_batch(budget.min(counts[0]), control)?;
    let mut holdout = empty_batch(budget.min(counts[1]), control)?;
    training.role = SampleRole::Training;
    holdout.role = SampleRole::Holdout;
    training.population_area_mm2 = areas[0];
    holdout.population_area_mm2 = areas[1];
    training.unqueried_area_mm2 = areas[1];
    holdout.unqueried_area_mm2 = areas[0];
    let retained = [
        training.samples.capacity().min(budget).min(counts[0]),
        holdout.samples.capacity().min(budget).min(counts[1]),
    ];
    let mut seen = [0usize; 2];
    for &sample in samples {
        control.charge_operations(1)?;
        let role = usize::from(cell_hash(sample.point)? & 8 != 0);
        let batch = if role == 0 {
            &mut training
        } else {
            &mut holdout
        };
        if retained[role] == 0 {
            continue;
        }
        let group = seen[role]
            .checked_mul(retained[role])
            .ok_or(GeometryStop::ResourceLimit)?
            / counts[role];
        seen[role] += 1;
        if group == batch.samples.len() {
            batch.samples.push(SurfaceSample {
                area_weight_mm2: 0.,
                ..sample
            });
        }
        batch.samples[group].area_weight_mm2 += sample.area_weight_mm2;
        batch.represented_area_mm2 += sample.area_weight_mm2;
    }
    Ok(SampleSplit { training, holdout })
}

/// Spatial jackknife label, shared by all points in a 1 mm cell.
///
/// # Errors
/// Returns numerical failure for non-finite/unrepresentable coordinates.
pub fn spatial_stratum(point: DVec3) -> Result<u8, GeometryStop> {
    u8::try_from(cell_hash(point)? & 7).map_err(|_| GeometryStop::Numerical)
}

pub(super) fn empty_batch(
    capacity: usize,
    control: &GeometryControl,
) -> Result<SampleBatch, GeometryStop> {
    let bytes = capacity
        .checked_mul(size_of::<SurfaceSample>())
        .and_then(|n| n.checked_add(64))
        .ok_or(GeometryStop::ResourceLimit)?;
    let memory = control.reserve(bytes)?;
    let mut samples = Vec::new();
    samples
        .try_reserve_exact(capacity)
        .map_err(|_| GeometryStop::ResourceLimit)?;
    Ok(SampleBatch {
        samples,
        population_area_mm2: 0.,
        represented_area_mm2: 0.,
        unqueried_area_mm2: 0.,
        role: SampleRole::Full,
        completion: Completion::Complete,
        _memory: memory,
    })
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub(super) fn cell_hash(point: DVec3) -> Result<u64, GeometryStop> {
    let mut hash = AREA_SAMPLE_SEED;
    for coordinate in point.to_array() {
        let floor = coordinate.floor();
        if !floor.is_finite() || floor.abs() >= 9_223_372_036_854_775_808. {
            return Err(GeometryStop::Numerical);
        }
        hash = mix_seed(hash ^ (floor as i64 as u64));
    }
    Ok(hash)
}

/// `SplitMix64` integer finalizer; fixed wrapping arithmetic defines stream ids.
pub fn mix_seed(mut value: u64) -> u64 {
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}
#[allow(clippy::cast_precision_loss)]
pub(super) fn uniform(state: &mut u64) -> f64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    ((mix_seed(*state) >> 12) as f64 + 0.5) / 4_503_599_627_370_496.
}
