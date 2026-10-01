//! The mesh editor tool window (the dental CAD "3D Data Editor" workflow).
//!
//! `mesh_editor_overlay` owns the window shell and the action vocabulary, with
//! `mesh_editor_groups` rendering its per-section controls and
//! `mesh_editor_session` the shared status and commit bar; `mesh_editor_icons`
//! draws the editor's own icon set.

pub(crate) mod mesh_editor_icons;
pub(crate) mod mesh_editor_overlay;
