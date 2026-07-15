#![forbid(unsafe_code)]

//! Robust finite-disc CSG fallback for closed triangle meshes.

use thiserror::Error;

/// A closed triangle mesh represented in `f64` coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct RobustMesh {
    /// One position per vertex.
    pub positions: Vec<[f64; 3]>,
    /// Triangle indices into [`Self::positions`].
    pub indices: Vec<u64>,
}

/// One logical output part from a robust separator operation.
#[derive(Clone, Debug, PartialEq)]
pub struct RobustMeshPart {
    /// Output positions in the robust kernel's precision.
    pub positions: Vec<[f64; 3]>,
    /// Triangle indices into [`Self::positions`].
    pub indices: Vec<u64>,
}

/// Physical component and cut-loop counts for one robust split.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RobustSplitReport {
    /// Number of disconnected physical solids composed into Part A.
    pub part_a_physical_components: usize,
    /// Number of disconnected physical solids composed into Part B.
    pub part_b_physical_components: usize,
    /// Number of separator boundary loops found on Part A.
    pub part_a_cut_loops: usize,
    /// Number of separator boundary loops found on Part B.
    pub part_b_cut_loops: usize,
}

/// Two logical parts left after removing a finite cylindrical separator disc.
#[derive(Clone, Debug, PartialEq)]
pub struct RobustSplit {
    /// The positive-normal part of the source mesh.
    pub part_a: RobustMeshPart,
    /// The negative-normal part of the source mesh.
    pub part_b: RobustMeshPart,
    /// Honest physical-component and cut-loop counts.
    pub report: RobustSplitReport,
}

/// Finite separator geometry in world space.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct SeparatorDisc {
    /// Centre of the disc's middle plane.
    pub center: [f64; 3],
    /// Unit direction from Part B towards Part A.
    pub normal: [f64; 3],
    /// Material removed along the normal.
    pub kerf_mm: f64,
    /// Disc radius in the perpendicular plane.
    pub radius_mm: f64,
}

/// Failure from the robust native CSG boundary.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum RobustCsgError {
    /// The caller passed malformed geometry.
    #[error("invalid robust CSG input: {reason}")]
    InvalidInput {
        /// Stable non-payload explanation.
        reason: String,
    },
    /// The native manifold kernel failed.
    #[error("robust CSG kernel failed: {reason}")]
    Kernel {
        /// Stable non-payload explanation.
        reason: String,
    },
    /// The difference did not leave the required physical components.
    #[error("robust CSG did not yield the required physical components: {components}")]
    UnexpectedComponents {
        /// Number of physical components returned by the difference.
        components: usize,
    },
    /// The disc missed the solid or only touched it.
    #[error("separator disc misses the prepared solid or only touches it")]
    SeparatorMiss,
    /// A physical result component still crosses the kerf slab.
    #[error("physical result component {component} spans the separator kerf")]
    KerfSpanningComponent {
        /// Stable result-component ordinal.
        component: usize,
    },
    /// A stored result intrudes into the finite separator volume.
    #[error("result mesh intersects the interior of the separator clearance")]
    SeparatorClearanceLost,
    /// An inward or mixed-winding shell was not accepted by the safe slice.
    #[error("inward shell {shell} is not supported by the robust prepared-solid slice: {reason}")]
    AmbiguousShellWinding {
        /// Stable shell ordinal.
        shell: usize,
        /// Stable explanation without geometry payload.
        reason: String,
    },
    /// The placement matrix is not finite and invertible.
    #[error("invalid robust CSG placement transform: {reason}")]
    InvalidTransform {
        /// Stable explanation without matrix payload.
        reason: String,
    },
}

pub(crate) fn invalid_input(reason: &str) -> RobustCsgError {
    RobustCsgError::InvalidInput {
        reason: reason.to_string(),
    }
}

pub(crate) fn kernel_error(error: impl std::fmt::Display) -> RobustCsgError {
    RobustCsgError::Kernel {
        reason: error.to_string(),
    }
}

pub(crate) fn flatten_positions(positions: &[[f64; 3]]) -> Vec<f64> {
    positions
        .iter()
        .flat_map(|position| position.iter().copied())
        .collect()
}

pub(crate) fn position_bounds(positions: &[[f64; 3]]) -> ([f64; 3], [f64; 3]) {
    positions.iter().fold(
        ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]),
        |(minimum, maximum), &position| {
            (
                [
                    minimum[0].min(position[0]),
                    minimum[1].min(position[1]),
                    minimum[2].min(position[2]),
                ],
                [
                    maximum[0].max(position[0]),
                    maximum[1].max(position[1]),
                    maximum[2].max(position[2]),
                ],
            )
        },
    )
}

mod disc_split;
mod native_mesh;
mod prepared_solid;
mod solid_compose;

pub use disc_split::{
    normalize_closed_mesh, split_prepared_with_separator_disc, split_with_separator_disc,
    validate_separator_clearance,
};
pub use prepared_solid::{prepare_robust_solid, PreparedRobustSolid};
