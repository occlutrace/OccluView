//! Shared UI layer for the viewer.
//!
//! Theme tokens and window visuals, the vendored icon renderer, the modal
//! surface, the contextual interaction hints, the viewport scale bar, and the
//! accessibility annotations. `live_viewport` and the public `invalidation`
//! module stay at the crate root: they are viewport state, not shared UI.

pub(crate) mod accessibility;
pub(crate) mod app_chrome;
pub(crate) mod icons;
pub(crate) mod interaction_hints;
pub(crate) mod modal_surface;
pub(crate) mod scale_bar;
pub(crate) mod ui_theme;
