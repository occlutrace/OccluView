//! Interactive sculpting of the edit-mode mesh.
//!
//! `sculpt_kernel` adapts the app's brush contract to the `occluview-sculpt`
//! kernel and mirrors the mesh attributes it does not carry. `sculpt_tool`
//! holds the brush state the viewport drives, and `sculpt_worker` runs the
//! kernel off the frame thread, with `sculpt_worker_loop` draining its command
//! queue.

pub(crate) mod sculpt_kernel;
pub(crate) mod sculpt_tool;
pub(crate) mod sculpt_worker;
