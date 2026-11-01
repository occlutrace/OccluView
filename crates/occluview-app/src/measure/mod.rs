//! Measuring the scene: rulers, angles, and the wall-thickness probe.
//!
//! `measure_tool` holds the measurement state machine and its geometry,
//! `measure_ruler` resolves ruler records into drawable segments,
//! `measure_overlay` paints them in the viewport, and `measure_draw` holds the
//! shared painting primitives used by both that overlay and the Section-panel
//! ruler.

pub(crate) mod measure_draw;
pub(crate) mod measure_overlay;
pub(crate) mod measure_ruler;
pub(crate) mod measure_tool;
