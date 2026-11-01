mod attributes;
mod cap;
pub(crate) mod clip;
mod operation;
mod planar_cap;
mod rims;
mod types;
mod validate;

pub use operation::{split_bridge, split_bridge_surface};
pub use types::{BridgeSplitReport, BridgeSplitRequest, BridgeSplitResult, SurfaceSplitResult};
pub use validate::{
    validate_bridge_split, validate_bridge_split_part, validate_bridge_split_request,
};

#[cfg(test)]
mod tests;
