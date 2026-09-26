#![allow(unexpected_cfgs)] // objc 0.2 emits cfg(cargo-clippy) in its macros.

//! Finder document-open callbacks installed on `winit`'s existing `AppKit` delegate.
//!
//! `winit` keeps its concrete application-delegate type private and checks that
//! type while running the event loop. Replacing `NSApplication.delegate` would
//! therefore break `winit`; adding the two optional document selectors to the
//! live delegate class preserves its identity and lifecycle.

#[cfg(test)]
use objc::declare::ClassDecl;
use objc::runtime::{
    class_addMethod, class_getInstanceMethod, object_getClass, Class, Imp, Object, Sel, BOOL, NO,
    YES,
};
use objc::{class, msg_send, sel, sel_impl};
use std::ffi::{c_char, CStr};
use std::os::unix::ffi::OsStringExt;
use std::path::PathBuf;

const OBJECT_PAIR_METHOD_ENCODING: &[u8] = b"v@:@@\0";

type ObjectPairMethod = unsafe extern "C" fn(*mut Object, Sel, *mut Object, *mut Object);

/// Add `AppKit`'s optional URL and legacy file-open callbacks to the live class.
///
/// The callbacks are both `void(id, SEL, id, id)`, represented by `v@:@@`.
/// `class_addMethod` only mutates this process's runtime class; the existing
/// `winit` delegate object, ivars, and lifecycle methods remain untouched.
fn add_open_methods(class: *mut Class) -> Result<(), &'static str> {
    let urls_selector = sel!(application:openURLs:);
    let files_selector = sel!(application:openFiles:);

    // Do not replace another owner's document handler or install only half of
    // the pair. Current winit has neither selector; a future implementation
    // should be reviewed as an explicit integration change.
    unsafe {
        if !class_getInstanceMethod(class, urls_selector).is_null()
            || !class_getInstanceMethod(class, files_selector).is_null()
        {
            return Err("the application delegate already owns a document-open selector");
        }

        let urls_imp: Imp = std::mem::transmute::<ObjectPairMethod, Imp>(application_open_urls);
        let files_imp: Imp = std::mem::transmute::<ObjectPairMethod, Imp>(application_open_files);
        let encoding = OBJECT_PAIR_METHOD_ENCODING.as_ptr().cast::<c_char>();

        if class_addMethod(class, urls_selector, urls_imp, encoding) != YES {
            return Err("could not add application:openURLs:");
        }
        if class_addMethod(class, files_selector, files_imp, encoding) != YES {
            return Err("could not add application:openFiles:");
        }
    }
    Ok(())
}

/// Install document handlers once `eframe` has created `winit`'s `NSApplication`.
pub(super) fn install() {
    unsafe {
        let application: *mut Object = msg_send![class!(NSApplication), sharedApplication];
        if application.is_null() {
            tracing::warn!("NSApplication is unavailable; Finder document opening is disabled");
            return;
        }
        let delegate: *mut Object = msg_send![application, delegate];
        if delegate.is_null() {
            tracing::warn!(
                "winit has no NSApplication delegate; Finder document opening is disabled"
            );
            return;
        }
        let delegate_class = object_getClass(delegate);
        if delegate_class.is_null() {
            tracing::warn!("winit NSApplication delegate has no runtime class");
            return;
        }

        match add_open_methods(delegate_class.cast_mut()) {
            Ok(()) => tracing::info!("installed Finder document-open callbacks on winit delegate"),
            Err(reason) => {
                tracing::warn!(reason, "Finder document-open callbacks were not installed");
            }
        }
    }
}

#[derive(Clone, Copy)]
enum OpenItemKind {
    FileUrl,
    FilePath,
}

/// Convert `AppKit`'s `NSArray` of file URLs or filesystem paths without lossy
/// UTF-8 conversion. The protocol's path-count limit is checked before reserve.
unsafe fn file_paths(items: *mut Object, kind: OpenItemKind) -> Option<Vec<PathBuf>> {
    if items.is_null() {
        return None;
    }

    let count: usize = unsafe { msg_send![items, count] };
    if count > super::protocol::MAX_REQUEST_PATHS {
        tracing::warn!(count, "Finder open request exceeds the path limit");
        return None;
    }

    let mut paths = Vec::with_capacity(count);
    for index in 0..count {
        let item: *mut Object = unsafe { msg_send![items, objectAtIndex: index] };
        if item.is_null() {
            continue;
        }
        if matches!(kind, OpenItemKind::FileUrl) {
            let is_file_url: BOOL = unsafe { msg_send![item, isFileURL] };
            if is_file_url == NO {
                continue;
            }
        }

        let representation: *const c_char = unsafe { msg_send![item, fileSystemRepresentation] };
        if representation.is_null() {
            tracing::warn!("AppKit returned a path without a filesystem representation");
            continue;
        }
        let bytes = unsafe { CStr::from_ptr(representation) }.to_bytes();
        if !bytes.is_empty() {
            paths.push(PathBuf::from(std::ffi::OsString::from_vec(bytes.to_vec())));
        }
    }
    Some(paths)
}

fn queue_open_paths(paths: Vec<PathBuf>) -> bool {
    if paths.is_empty() {
        return false;
    }
    let request = super::OpenRequest {
        paths,
        activation_token: None,
    };
    match super::write_open_request(&request) {
        Ok(()) => true,
        Err(error) => {
            tracing::warn!(?error, "could not queue Finder document-open request");
            false
        }
    }
}

unsafe extern "C" fn application_open_urls(
    _delegate: *mut Object,
    _selector: Sel,
    _application: *mut Object,
    urls: *mut Object,
) {
    let paths = unsafe { file_paths(urls, OpenItemKind::FileUrl) };
    if !paths.is_some_and(queue_open_paths) {
        tracing::warn!("Finder URL event contained no queueable file paths");
    }
}

unsafe extern "C" fn application_open_files(
    _delegate: *mut Object,
    _selector: Sel,
    application: *mut Object,
    filenames: *mut Object,
) {
    let queued =
        unsafe { file_paths(filenames, OpenItemKind::FilePath) }.is_some_and(queue_open_paths);
    let reply = if queued { 0usize } else { 2usize };
    if !application.is_null() {
        let _: () = unsafe { msg_send![application, replyToOpenOrPrint: reply] };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finder_open_methods_can_be_added_without_replacing_the_delegate_class() {
        let declaration = ClassDecl::new("OccluViewFinderOpenTestDelegate", class!(NSObject));
        assert!(
            declaration.is_some(),
            "the test delegate class name must be unique"
        );
        let Some(declaration) = declaration else {
            return;
        };
        let class = declaration.register();
        assert!(add_open_methods(std::ptr::from_ref::<Class>(class).cast_mut()).is_ok());
        unsafe {
            assert!(!class_getInstanceMethod(class, sel!(application:openURLs:)).is_null());
            assert!(!class_getInstanceMethod(class, sel!(application:openFiles:)).is_null());
        }
    }
}
