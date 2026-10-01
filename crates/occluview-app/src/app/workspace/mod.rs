//! App-level types for coordinating multiple independent scene documents.
//!
//! A core `Scene` remains the contents of one dental case. This module owns
//! only workspace identity, layout, input ownership, and user commands.

pub(crate) mod commands;
pub(crate) mod history;
pub(crate) mod id;
pub(crate) mod input;
pub(crate) mod layout;
pub(crate) mod loading;
pub(in crate::app) mod state;
mod transfer;
