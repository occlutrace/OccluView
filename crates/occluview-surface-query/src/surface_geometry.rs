//! Exact triangle projection used by the surface index.

// Feature information feeds border and pseudonormal queries, while the
// geometric closest-point calculation is shared by mesh kernels.
pub(crate) use occluview_geometry_math::{
    closest_feature_on_triangle, ClosestTriangleFeature as Feature,
};
