use thiserror::Error;

/// Errors raised by mesh edit validation and preparation helpers.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum MeshEditError {
    /// Face-edit commands do not apply to point clouds.
    #[error("mesh edit operations do not support point clouds")]
    UnsupportedPointCloud,

    /// A face-selection mask did not match the triangle count.
    #[error("selection length {actual} does not match triangle count {expected}")]
    InvalidSelectionLength {
        /// Expected triangle count.
        expected: usize,
        /// Actual selection mask length.
        actual: usize,
    },

    /// The mesh or mesh-like data is malformed.
    #[error("malformed mesh: {reason}")]
    MalformedMesh {
        /// Human-readable validation reason.
        reason: String,
    },

    /// The edit options are invalid for the requested operation.
    #[error("invalid mesh edit options: {reason}")]
    InvalidOptions {
        /// Human-readable validation reason.
        reason: String,
    },
}

/// Errors raised while preparing or executing a bridge split.
#[derive(Clone, Debug, Error, PartialEq)]
pub enum BridgeSplitError {
    /// The source buffers failed shared mesh validation.
    #[error(transparent)]
    Mesh(#[from] MeshEditError),

    /// The disc plane or kerf settings are not finite and physically meaningful.
    #[error("invalid bridge split request: {reason}")]
    InvalidRequest {
        /// Stable, user-safe explanation of the invalid setting.
        reason: String,
    },

    /// There are no triangles to split.
    #[error("bridge split requires a non-empty triangle mesh")]
    EmptyInput,

    /// One or more faces collapse after exact geometric seam recovery.
    #[error("bridge split input contains {faces} degenerate face(s)")]
    DegenerateInput {
        /// Number of faces with fewer than three distinct geometric corners.
        faces: usize,
    },

    /// The source represents more than one edge-connected physical component.
    #[error("bridge split requires one connected mesh, found {components}")]
    DisconnectedInput {
        /// Number of edge-connected components.
        components: usize,
    },

    /// The source is open, non-manifold, or inconsistently oriented.
    #[error(
        "bridge split requires a closed oriented manifold (boundary edges: {boundary_edges}, non-manifold edges: {non_manifold_edges}, inconsistent edges: {inconsistent_winding_edges}, non-manifold vertices: {non_manifold_vertices})"
    )]
    OpenOrNonManifold {
        /// Undirected edges incident to only one face.
        boundary_edges: usize,
        /// Undirected edges incident to more than two faces.
        non_manifold_edges: usize,
        /// Two-face edges whose directed uses do not oppose each other.
        inconsistent_winding_edges: usize,
        /// Vertices whose incident face fan is disconnected.
        non_manifold_vertices: usize,
    },

    /// The kerf slab does not intersect the target at all.
    #[error("separator disc does not intersect the bridge")]
    NoIntersection,

    /// The slab only touches the target and cannot produce two volumetric parts.
    #[error("separator disc only touches the bridge; move it farther into the connector")]
    TangentContact,

    /// The auto-fitted disc would exceed the configured safety ceiling.
    #[error(
        "required separator radius {required_radius_mm:.3} mm exceeds the {max_radius_mm:.3} mm limit"
    )]
    DiscLimitExceeded {
        /// Radius required to cover every removed slab polygon plus margin.
        required_radius_mm: f32,
        /// Caller-provided maximum disc radius.
        max_radius_mm: f32,
    },

    /// The chosen disc is smaller than the complete kerf cross-section.
    #[error(
        "separator disc radius {disc_radius_mm:.3} mm is below the {required_radius_mm:.3} mm needed to split the bridge"
    )]
    DiscTooSmall {
        /// Radius selected by the operator.
        disc_radius_mm: f32,
        /// Minimum radius that spans every removed slab polygon plus margin.
        required_radius_mm: f32,
    },

    /// Generated cut segments do not form only simple closed loops.
    #[error("damaged bridge split rim: {reason}")]
    DamagedCutRim {
        /// Deterministic user-safe reason for refusing the rim.
        reason: String,
    },

    /// A complete cap could not be produced for every cut loop.
    #[error("bridge split cap failed: {reason}")]
    CapFailed {
        /// Deterministic user-safe reason for refusing the cap.
        reason: String,
    },

    /// A capped side failed final manufacturability validation.
    #[error("invalid bridge split output {side}: {reason}")]
    InvalidOutput {
        /// Stable side label (`Part A` or `Part B`).
        side: &'static str,
        /// Validation failure without raw mesh payloads.
        reason: String,
    },

    /// Quantized output geometry does not preserve the requested minimum gap.
    #[error("bridge split gap {observed_mm:.4} mm is below requested {requested_mm:.4} mm")]
    SeparationViolation {
        /// Minimum observed separation along the disc normal.
        observed_mm: f32,
        /// Requested kerf.
        requested_mm: f32,
    },
}

#[cfg(test)]
mod error_contract {
    use super::*;

    /// Names every variant so a new one cannot be added without a case here.
    fn mesh_edit_variant_name(error: &MeshEditError) -> &'static str {
        match error {
            MeshEditError::UnsupportedPointCloud => "UnsupportedPointCloud",
            MeshEditError::InvalidSelectionLength { .. } => "InvalidSelectionLength",
            MeshEditError::MalformedMesh { .. } => "MalformedMesh",
            MeshEditError::InvalidOptions { .. } => "InvalidOptions",
        }
    }

    /// Names every variant so a new one cannot be added without a case here.
    fn bridge_split_variant_name(error: &BridgeSplitError) -> &'static str {
        match error {
            BridgeSplitError::Mesh(_) => "Mesh",
            BridgeSplitError::InvalidRequest { .. } => "InvalidRequest",
            BridgeSplitError::EmptyInput => "EmptyInput",
            BridgeSplitError::DegenerateInput { .. } => "DegenerateInput",
            BridgeSplitError::DisconnectedInput { .. } => "DisconnectedInput",
            BridgeSplitError::OpenOrNonManifold { .. } => "OpenOrNonManifold",
            BridgeSplitError::NoIntersection => "NoIntersection",
            BridgeSplitError::TangentContact => "TangentContact",
            BridgeSplitError::DiscLimitExceeded { .. } => "DiscLimitExceeded",
            BridgeSplitError::DiscTooSmall { .. } => "DiscTooSmall",
            BridgeSplitError::DamagedCutRim { .. } => "DamagedCutRim",
            BridgeSplitError::CapFailed { .. } => "CapFailed",
            BridgeSplitError::InvalidOutput { .. } => "InvalidOutput",
            BridgeSplitError::SeparationViolation { .. } => "SeparationViolation",
        }
    }

    #[test]
    fn every_mesh_edit_error_variant_renders_an_operator_message() {
        let variants = [
            MeshEditError::UnsupportedPointCloud,
            MeshEditError::InvalidSelectionLength {
                expected: 3,
                actual: 2,
            },
            MeshEditError::MalformedMesh {
                reason: "index out of range".to_string(),
            },
            MeshEditError::InvalidOptions {
                reason: "kerf must be positive".to_string(),
            },
        ];
        for variant in &variants {
            assert!(!mesh_edit_variant_name(variant).is_empty());
            let message = variant.to_string();
            assert!(!message.trim().is_empty(), "empty message for {variant:?}");
            assert!(
                !message.contains("occlu-") && !message.contains("occlu_"),
                "operator message names an internal crate: {message:?}"
            );
        }
    }

    #[test]
    fn every_bridge_split_error_variant_renders_an_operator_message() {
        let variants = [
            BridgeSplitError::Mesh(MeshEditError::UnsupportedPointCloud),
            BridgeSplitError::InvalidRequest {
                reason: "kerf must be positive".to_string(),
            },
            BridgeSplitError::EmptyInput,
            BridgeSplitError::DegenerateInput { faces: 1 },
            BridgeSplitError::DisconnectedInput { components: 2 },
            BridgeSplitError::OpenOrNonManifold {
                boundary_edges: 1,
                non_manifold_edges: 2,
                inconsistent_winding_edges: 3,
                non_manifold_vertices: 4,
            },
            BridgeSplitError::NoIntersection,
            BridgeSplitError::TangentContact,
            BridgeSplitError::DiscLimitExceeded {
                required_radius_mm: 2.0,
                max_radius_mm: 1.0,
            },
            BridgeSplitError::DiscTooSmall {
                disc_radius_mm: 1.0,
                required_radius_mm: 2.0,
            },
            BridgeSplitError::DamagedCutRim {
                reason: "open rim".to_string(),
            },
            BridgeSplitError::CapFailed {
                reason: "cap did not close".to_string(),
            },
            BridgeSplitError::InvalidOutput {
                side: "Part A",
                reason: "not manifold".to_string(),
            },
            BridgeSplitError::SeparationViolation {
                observed_mm: 0.1,
                requested_mm: 0.2,
            },
        ];
        for variant in &variants {
            assert!(!bridge_split_variant_name(variant).is_empty());
            let message = variant.to_string();
            assert!(!message.trim().is_empty(), "empty message for {variant:?}");
            assert!(
                !message.contains("occlu-") && !message.contains("occlu_"),
                "operator message names an internal crate: {message:?}"
            );
        }
    }
}
