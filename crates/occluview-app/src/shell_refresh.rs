//! Telling Explorer that this build's file associations changed.
//!
//! One `SHChangeNotify` call, kept here rather than imported from
//! `occluview-shell`. That crate is the COM surface -- thumbnail provider,
//! preview handler, registration -- and the viewer needed exactly this one
//! line from it, which linked all of it into the GUI binary.

/// Tell Explorer that file associations and shell handlers changed.
#[cfg(windows)]
pub(crate) fn notify_shell_associations_changed() {
    use windows::Win32::UI::Shell::{SHChangeNotify, SHCNE_ASSOCCHANGED, SHCNF_IDLIST};

    // SAFETY: SHChangeNotify accepts null item pointers for SHCNE_ASSOCCHANGED.
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
}
