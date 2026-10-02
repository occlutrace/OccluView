//! Atomic layer moves and their linked workspace history navigation.
//!
//! The payload keeps `SceneMesh` values, which retain the original `Arc<Mesh>`.
//! A transfer therefore moves layer ownership and metadata without copying the
//! scan's vertices or texture buffers.

mod apply;
mod command;
mod navigate;
mod record;
#[cfg(test)]
mod tests;
