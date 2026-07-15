mod attributes;
mod cap;
pub(crate) mod clip;
mod operation;
mod rims;
mod types;
mod validate;

pub use operation::split_bridge;
pub use types::{BridgeSplitReport, BridgeSplitRequest, BridgeSplitResult};
pub use validate::{
    validate_bridge_split, validate_bridge_split_part, validate_bridge_split_request,
};

#[cfg(test)]
mod tests;
