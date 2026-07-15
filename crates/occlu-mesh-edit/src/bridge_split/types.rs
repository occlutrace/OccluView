use crate::MeshEditBuffers;
use glam::Vec3;

/// World- or adapter-normalized disc placement for one bridge split.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct BridgeSplitRequest {
    /// Center of the separator disc, in mesh units (millimetres in OccluView).
    pub center: Vec3,
    /// Disc normal. The kernel normalizes a finite non-zero value once.
    pub normal: Vec3,
    /// Material removed between the two resulting cap planes.
    pub kerf_mm: f32,
    /// Radius of the physical separator disc selected by the operator.
    pub disc_radius_mm: f32,
    /// Safety ceiling for the selected finite separator disc.
    pub max_disc_radius_mm: f32,
}

/// Stable operation statistics suitable for UI status and diagnostics.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BridgeSplitReport {
    /// Input triangle count.
    pub input_triangles: usize,
    /// Output triangle count in Part A.
    pub part_a_triangles: usize,
    /// Output triangle count in Part B.
    pub part_b_triangles: usize,
    /// Applied kerf in mesh units.
    pub kerf_mm: f32,
    /// Applied finite disc radius in mesh units.
    pub disc_radius_mm: f32,
    /// Smallest radius that fully spans the removed kerf slab.
    pub required_disc_radius_mm: f32,
    /// Number of simple cut loops capped on Part A.
    pub part_a_cut_loops: usize,
    /// Number of simple cut loops capped on Part B.
    pub part_b_cut_loops: usize,
}

/// Two validated bridge parts produced atomically by the split kernel.
#[derive(Clone, Debug, PartialEq)]
pub struct BridgeSplitResult {
    /// Geometry on the positive side of the kerf slab.
    pub part_a: MeshEditBuffers,
    /// Geometry on the negative side of the kerf slab.
    pub part_b: MeshEditBuffers,
    /// Deterministic operation statistics.
    pub report: BridgeSplitReport,
}
