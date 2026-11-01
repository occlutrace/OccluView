//! App-level types for coordinating multiple independent scene documents.
//!
//! A core `Scene` remains the contents of one dental case. This module owns
//! only workspace identity, layout, input ownership, and user commands.

pub(crate) mod commands;
mod dialogs;
pub(crate) mod history;
pub(crate) mod id;
pub(crate) mod input;
mod layer_drag;
pub(crate) mod layout;
pub(crate) mod loading;
mod pane;
mod pane_geometry;
mod pointer;
mod show;
pub(in crate::app) mod state;
#[cfg(test)]
mod tests;
mod transfer;
