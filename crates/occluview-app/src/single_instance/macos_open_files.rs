//! Finder document-open callbacks installed on `winit`'s existing `AppKit` delegate.
//!
//! `winit` keeps its concrete application-delegate type private and checks that
//! type while running the event loop. Adding the two optional document selectors
//! to the live delegate class preserves its identity and lifecycle.
//!
//! When Finder launches the app to open a document, `AppKit` delivers it after
//! `applicationWillFinishLaunching:` and before `applicationDidFinishLaunching:`.
//! eframe creates the window, and runs the app creator, only after the latter,
//! so the selectors are added from a `WillFinishLaunching` observer registered
//! before the event loop runs.

use objc2::ffi;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Imp, Sel};
use objc2::{define_class, msg_send, sel, ClassType, MainThreadMarker, MainThreadOnly, Message};
use objc2_app_kit::{
    NSApplication, NSApplicationDelegateReply, NSApplicationWillFinishLaunchingNotification,
};
use objc2_foundation::{
    NSArray, NSNotification, NSNotificationCenter, NSObject, NSObjectProtocol, NSString, NSURL,
};
use std::ffi::{c_char, CStr};
use std::os::unix::ffi::OsStringExt;
use std::path::PathBuf;
use std::ptr::NonNull;
use std::sync::Once;

/// Adds the document selectors at most once, whichever caller comes first.
static INSTALL: Once = Once::new();
/// Registers the observer once before AppKit dispatches Finder's launch files.
static OBSERVER_REGISTRATION: Once = Once::new();

/// Both optional AppKit callbacks return void and accept two object arguments.
const OBJECT_PAIR_METHOD_ENCODING: &[u8] = b"v@:@@\0";

define_class!(
    // SAFETY:
    // - `NSObject` has no subclassing requirements.
    // - The observer has no Rust fields or destructor.
    #[unsafe(super = NSObject)]
    #[name = "OccluViewLaunchObserver"]
    #[thread_kind = MainThreadOnly]
    struct LaunchObserver;

    // SAFETY: `NSObjectProtocol` has no safety requirements.
    unsafe impl NSObjectProtocol for LaunchObserver {}

    impl LaunchObserver {
        #[unsafe(method(occluviewWillFinishLaunching:))]
        fn will_finish_launching(&self, _notification: &NSNotification) {
            install();
        }
    }
);

/// Add `AppKit`'s optional URL and legacy file-open callbacks to the live class.
///
/// The callbacks share the `void(id, SEL, id, id)` encoding `v@:@@`. The
/// typed function signatures keep their argument types checked by Rust.
fn add_open_methods(class: &AnyClass) -> Result<(), &'static str> {
    let urls_selector = sel!(application:openURLs:);
    let files_selector = sel!(application:openFiles:);

    // Do not replace another owner's document handler or install only half of
    // the pair. Current winit has neither selector; a future implementation
    // should be reviewed as an explicit integration change.
    if class.instance_method(urls_selector).is_some()
        || class.instance_method(files_selector).is_some()
    {
        return Err("the application delegate already owns a document-open selector");
    }

    let urls_imp = unsafe {
        std::mem::transmute::<
            unsafe extern "C-unwind" fn(&AnyObject, Sel, &NSApplication, &NSArray<NSURL>),
            Imp,
        >(application_open_urls)
    };
    let files_imp = unsafe {
        std::mem::transmute::<
            unsafe extern "C-unwind" fn(&AnyObject, Sel, &NSApplication, &NSArray<NSString>),
            Imp,
        >(application_open_files)
    };
    let class_ptr = std::ptr::from_ref(class).cast_mut();
    let encoding = OBJECT_PAIR_METHOD_ENCODING.as_ptr().cast::<c_char>();

    // SAFETY: The class is the live winit delegate and each implementation has
    // the AppKit selector's verified object-pair ABI.
    unsafe {
        if !ffi::class_addMethod(class_ptr, urls_selector, urls_imp, encoding).as_bool() {
            return Err("could not add application:openURLs:");
        }
        if !ffi::class_addMethod(class_ptr, files_selector, files_imp, encoding).as_bool() {
            return Err("could not add application:openFiles:");
        }
    }
    Ok(())
}

/// Arrange for [`install`] to run as `NSApplication` finishes launching, before
/// `AppKit` delivers the documents of a Finder launch. Call before the event
/// loop runs; `winit` has set its delegate by the time the notification fires.
pub(super) fn install_when_launching() {
    OBSERVER_REGISTRATION.call_once(register_launch_observer);
}

fn register_launch_observer() {
    if MainThreadMarker::new().is_none() {
        tracing::warn!("launch observer must be registered on the main thread");
        return;
    }

    // SAFETY: `new` is inherited from NSObject and returns a retained instance
    // of the registered observer class.
    let observer: Retained<LaunchObserver> = unsafe { msg_send![LaunchObserver::class(), new] };
    let center = NSNotificationCenter::defaultCenter();
    // The notification center does not retain selector-based observers. Keep
    // this instance alive for the process after registering it.
    unsafe {
        center.addObserver_selector_name_object(
            observer.as_super().as_super(),
            sel!(occluviewWillFinishLaunching:),
            Some(NSApplicationWillFinishLaunchingNotification),
            None,
        );
    }
    std::mem::forget(observer);
}

/// Add the document handlers to `winit`'s `NSApplication` delegate. Runs at
/// most once; later calls do nothing.
pub(super) fn install() {
    INSTALL.call_once(install_now);
}

fn install_now() {
    let Some(main_thread) = MainThreadMarker::new() else {
        tracing::warn!("Finder document handlers must be installed on the main thread");
        return;
    };
    let application = NSApplication::sharedApplication(main_thread);
    let Some(delegate) = application.delegate() else {
        tracing::warn!("winit has no NSApplication delegate; Finder document opening is disabled");
        return;
    };

    let delegate_object: &AnyObject = (&*delegate).as_ref();
    match add_open_methods(delegate_object.class()) {
        Ok(()) => tracing::info!("installed Finder document-open callbacks on winit delegate"),
        Err(reason) => {
            tracing::warn!(reason, "Finder document-open callbacks were not installed");
        }
    }
}

/// Convert `AppKit`'s array of file URLs or filesystem paths without lossy UTF-8
/// conversion. The protocol's path-count limit is checked before reserve.
fn file_paths<T: Message>(
    items: &NSArray<T>,
    file_representation: impl Fn(&T) -> Option<NonNull<c_char>>,
) -> Option<Vec<PathBuf>> {
    let count = items.count() as usize;
    if count > super::protocol::MAX_REQUEST_PATHS {
        tracing::warn!(count, "Finder open request exceeds the path limit");
        return None;
    }

    let mut paths = Vec::with_capacity(count);
    for index in 0..count {
        let item = items.objectAtIndex(index as _);
        let Some(representation) = file_representation(&item) else {
            continue;
        };
        // SAFETY: Foundation returns a NUL-terminated filesystem path for the
        // lifetime of the retained array item.
        let bytes = unsafe { CStr::from_ptr(representation.as_ptr()) }.to_bytes();
        if !bytes.is_empty() {
            paths.push(PathBuf::from(std::ffi::OsString::from_vec(bytes.to_vec())));
        }
    }
    Some(paths)
}

fn file_url_representation(url: &NSURL) -> Option<NonNull<c_char>> {
    url.isFileURL().then(|| url.fileSystemRepresentation())
}

fn path_representation(path: &NSString) -> Option<NonNull<c_char>> {
    Some(path.fileSystemRepresentation())
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

unsafe extern "C-unwind" fn application_open_urls(
    _delegate: &AnyObject,
    _selector: Sel,
    _application: &NSApplication,
    urls: &NSArray<NSURL>,
) {
    let paths = file_paths(urls, file_url_representation);
    if !paths.is_some_and(queue_open_paths) {
        tracing::warn!("Finder URL event contained no queueable file paths");
    }
}

unsafe extern "C-unwind" fn application_open_files(
    _delegate: &AnyObject,
    _selector: Sel,
    application: &NSApplication,
    filenames: &NSArray<NSString>,
) {
    let queued = file_paths(filenames, path_representation).is_some_and(queue_open_paths);
    let reply = if queued {
        NSApplicationDelegateReply::Success
    } else {
        NSApplicationDelegateReply::Failure
    };
    application.replyToOpenOrPrint(reply);
}

#[cfg(test)]
mod tests {
    use super::*;
    use objc2::runtime::ClassBuilder;

    #[test]
    fn finder_open_methods_keep_the_appkit_callback_encoding() {
        let builder = ClassBuilder::new(c"OccluViewFinderOpenTestDelegate", NSObject::class())
            .expect("the Finder callback test class name is unique");
        let class = builder.register();
        add_open_methods(class).expect("both Finder callbacks install");

        for selector in [sel!(application:openURLs:), sel!(application:openFiles:)] {
            let method = class
                .instance_method(selector)
                .expect("the test delegate owns the Finder callback");
            assert_eq!(method.return_type().to_str().unwrap(), "v");
            for (index, expected) in ["@", ":", "@", "@"].into_iter().enumerate() {
                assert_eq!(
                    method.argument_type(index).unwrap().to_str().unwrap(),
                    expected
                );
            }
        }
    }
}
