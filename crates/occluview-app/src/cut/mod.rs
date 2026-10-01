//! Slicing the scene open and probing the section.
//!
//! The interactive cut disc: `cut_manipulator` is its gesture state machine,
//! `cut_geometry` the stateless geometry under it, `cut_overlay` paints it, and
//! `cut_tool` adapts it to the viewport. `section_view` and `cut_ruler` render
//! the resulting section with its in-slice ruler, and `probe_section` bridges
//! the wall-thickness probe to that view.

pub(crate) mod cut_geometry;
pub(crate) mod cut_manipulator;
#[cfg(test)]
mod cut_manipulator_hostile_tests;
pub(crate) mod cut_overlay;
pub(crate) mod cut_ruler;
pub(crate) mod cut_tool;
pub(crate) mod probe_section;
pub(crate) mod section_view;
