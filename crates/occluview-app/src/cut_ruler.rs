//! In-slice measuring ruler and the docked Section panel.
//!
//! Geometry stays in `model`; panel layout, input, and drawing stay in `panel`.

mod model;
mod panel;

pub(crate) use model::{CutRuler, SliceBasis, SliceCam, SlicePlaneMap};
pub(crate) use panel::{
    section_image_rect_for, section_panel_contains, section_panel_rect,
    show_section_panel_with_basis, SectionDisplay, SectionPanelCommand, SectionRender,
    SliceMeasureMode,
};
