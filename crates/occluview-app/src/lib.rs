//! `occluview-app` library boundary.
//!
//! Testable composition helpers without windowing, GPU, or platform effects.
//! The binary entry point (`main.rs`) keeps bootstrap wiring; everything here
//! is pure and UI-neutral so behavior can be pinned by unit tests.

pub mod invalidation;
mod startup;

pub use startup::{
    file_extensions, parse_args, parse_args_from, should_append_incoming_open_state, StartupArgs,
};
