//! Contact readings between opposing surfaces.
//!
//! `contact` holds the clinical contact state, the per-layer readings and the
//! request types; `contact_worker` runs the measurement off the frame thread.

pub(crate) mod contact;
#[cfg(test)]
mod contact_render_tests;
pub(crate) mod contact_worker;
