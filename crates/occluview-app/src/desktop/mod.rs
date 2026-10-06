//! Desktop integration around the viewer.
//!
//! The single-instance handoff, the Windows shell surfaces (Jump List,
//! association refresh), the recent-file list, the per-user state paths, and
//! the update notice. `startup` and `app_bootstrap` stay at the crate root:
//! they are the composition root, not one desktop concern.

pub(crate) mod app_paths;
#[cfg(windows)]
pub(crate) mod jump_list;
pub(crate) mod jump_list_model;
pub(crate) mod recent_files;
#[cfg(windows)]
pub(crate) mod shell_refresh;
pub(crate) mod single_instance;
pub(crate) mod system_info;
pub(crate) mod update_notice;
