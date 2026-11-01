use std::path::{Path, PathBuf};

/// Identity of one activation of a [`DeferredSource`].
///
/// Every initialize/clear path advances the source generation. A stream
/// loader captures this token before it releases all handler borrows for an
/// external `IStream` call, then checks it again before installing any bytes,
/// preview scene, or oversize state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SourceToken(u64);

#[derive(Debug)]
pub(crate) struct DeferredSource<S> {
    pending_stream: Option<S>,
    path: Option<PathBuf>,
    extension: Option<String>,
    generation: u64,
}

impl<S> Default for DeferredSource<S> {
    fn default() -> Self {
        Self {
            pending_stream: None,
            path: None,
            extension: None,
            generation: 0,
        }
    }
}

impl<S> DeferredSource<S> {
    fn advance_generation(&mut self) {
        // Wrapping avoids a debug-overflow panic. Reaching the wrap point
        // would require 2^64 activations in one shell handler object.
        self.generation = self.generation.wrapping_add(1);
    }

    /// Stage a stream activation.
    ///
    /// Returns any older pending stream so the caller can drop the retired
    /// COM reference after its handler `RefCell` borrow has ended. A custom
    /// `IStream::Release` can reenter Initialize/Unload just like `Seek`,
    /// `Stat`, or `Read`.
    pub(crate) fn initialize_stream(&mut self, stream: S) -> Option<S> {
        self.advance_generation();
        let retired = self.pending_stream.replace(stream);
        self.path = None;
        self.extension = None;
        retired
    }

    /// Stage a path activation.
    ///
    /// Returns any older pending stream so the caller can drop the retired
    /// COM reference after its handler `RefCell` borrow has ended.
    pub(crate) fn initialize_path(&mut self, path: PathBuf, extension: Option<String>) -> Option<S> {
        self.advance_generation();
        let retired = self.pending_stream.take();
        self.path = Some(path);
        self.extension = extension;
        retired
    }

    pub(crate) fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub(crate) fn extension(&self) -> Option<&str> {
        self.extension.as_deref()
    }

    /// Return the identity of the currently staged activation.
    pub(crate) fn token(&self) -> SourceToken {
        SourceToken(self.generation)
    }

    /// True while `token` still names the current activation.
    pub(crate) fn is_current(&self, token: SourceToken) -> bool {
        self.generation == token.0
    }

    /// Take the staged stream out of this source and return it with a clone of
    /// the current extension metadata.
    ///
    /// The caller can release the `RefCell` borrow immediately, call the
    /// external `IStream` methods without holding handler state borrowed, and
    /// use the returned token to fence the result before installing it.
    pub(crate) fn take_pending_stream(&mut self) -> Option<(S, Option<String>, SourceToken)> {
        let stream = self.pending_stream.take()?;
        // Clone rather than take: path/extension metadata remains meaningful
        // after a stream copy, matching the old lazy-load API.
        let extension = self.extension.clone();
        Some((stream, extension, self.token()))
    }

    /// Revoke the current activation and return any pending stream.
    ///
    /// The caller owns the retired COM reference and must drop it after the
    /// handler `RefCell` borrow has ended.
    pub(crate) fn clear_all(&mut self) -> Option<S> {
        self.advance_generation();
        let retired = self.pending_stream.take();
        self.path = None;
        self.extension = None;
        retired
    }

    #[cfg(test)]
    pub(crate) fn has_pending_stream(&self) -> bool {
        self.pending_stream.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::DeferredSource;
    use std::cell::{Cell, RefCell};
    use std::path::{Path, PathBuf};
    use std::rc::{Rc, Weak};

    #[test]
    fn initialize_stream_stages_lazy_source_without_path_metadata() {
        let mut source = DeferredSource::<u8>::default();
        let retired = source.initialize_stream(7_u8);

        assert!(retired.is_none());
        assert!(source.has_pending_stream());
        assert!(source.path().is_none());
        assert!(source.extension().is_none());
    }

    #[test]
    fn initialize_path_clears_pending_stream_and_sets_extension() {
        let mut source = DeferredSource::<u8>::default();
        assert!(source.initialize_stream(7_u8).is_none());
        let retired =
            source.initialize_path(PathBuf::from("scan/model.obj"), Some("obj".to_string()));

        assert!(retired.is_some(), "the old pending stream is returned, not dropped");
        assert!(!source.has_pending_stream());
        assert_eq!(source.path(), Some(Path::new("scan/model.obj")));
        assert_eq!(source.extension(), Some("obj"));
    }

    #[test]
    fn pending_stream_is_consumed_only_by_explicit_lazy_load() {
        let mut source = DeferredSource::<u8>::default();
        assert!(source.initialize_stream(11_u8).is_none());

        let (stream, extension, token) = source
            .take_pending_stream()
            .expect("the staged stream is available exactly once");
        assert_eq!(stream, 11_u8);
        assert_eq!(extension, None);
        assert!(source.is_current(token));
        assert!(!source.has_pending_stream());
        assert!(source.take_pending_stream().is_none());
    }

    #[test]
    fn take_pending_stream_returns_owned_extension_and_keeps_metadata() {
        let mut source = DeferredSource::<u8>::default();
        source.pending_stream = Some(11_u8);
        source.extension = Some("obj".to_string());

        let (stream, extension, _) = source.take_pending_stream().expect("staged stream");

        assert_eq!(stream, 11_u8);
        assert_eq!(extension.as_deref(), Some("obj"));
        assert_eq!(source.extension(), Some("obj"));
    }

    #[test]
    fn path_metadata_survives_a_non_stream_take() {
        let mut source = DeferredSource::<u8>::default();
        assert!(source
            .initialize_path(PathBuf::from("scan/model.obj"), Some("obj".to_string()))
            .is_none());

        assert!(source.take_pending_stream().is_none());

        assert_eq!(source.path(), Some(Path::new("scan/model.obj")));
        assert_eq!(source.extension(), Some("obj"));
    }

    #[test]
    fn clear_all_returns_any_pending_stream_and_revokes_identity() {
        let mut source = DeferredSource::<u8>::default();
        assert!(source.initialize_stream(7_u8).is_none());
        let token = source.token();
        let retired = source.clear_all();

        assert_eq!(retired, Some(7_u8));
        assert!(!source.has_pending_stream());
        assert!(source.path().is_none());
        assert!(source.extension().is_none());
        assert!(!source.is_current(token));
    }

    #[test]
    fn a_new_stream_activation_revokes_an_older_result_token() {
        let mut source = DeferredSource::<u8>::default();
        assert!(source.initialize_stream(1_u8).is_none());
        let old_token = source.token();
        let (old_stream, _, _) = source.take_pending_stream().expect("staged stream");
        assert_eq!(old_stream, 1_u8);

        assert!(source.initialize_stream(2_u8).is_none());
        let new_token = source.token();

        assert_ne!(old_token, new_token);
        assert!(!source.is_current(old_token));
        assert!(source.is_current(new_token));
        assert_eq!(
            source.take_pending_stream().map(|(stream, _, _)| stream),
            Some(2_u8)
        );
    }

    #[test]
    fn unload_revokes_a_taken_stream_result_token() {
        let mut source = DeferredSource::<u8>::default();
        assert!(source.initialize_stream(3_u8).is_none());
        let token = source.token();
        let (taken_stream, _, _) = source.take_pending_stream().expect("staged stream");
        assert_eq!(taken_stream, 3_u8);

        let retired = source.clear_all();

        assert!(retired.is_none());
        assert!(!source.is_current(token));
        assert!(!source.has_pending_stream());
    }

    #[test]
    fn a_stream_result_can_be_installed_only_while_its_token_is_current() {
        let mut source = DeferredSource::<u8>::default();
        assert!(source.initialize_stream(1_u8).is_none());
        let (_, _, token) = source.take_pending_stream().expect("staged stream");

        let mut installed = None;
        if source.is_current(token) {
            installed = Some(1_u8);
        }
        assert_eq!(installed, Some(1_u8));

        assert!(source.initialize_stream(2_u8).is_none());
        let mut late_installed = None;
        if source.is_current(token) {
            late_installed = Some(1_u8);
        }
        assert_eq!(late_installed, None);
    }

    #[test]
    fn path_and_stream_activations_do_not_share_identity() {
        let mut source = DeferredSource::<u8>::default();
        assert!(source.initialize_stream(5_u8).is_none());
        let stream_token = source.token();
        let retired =
            source.initialize_path(PathBuf::from("scan/model.obj"), Some("obj".to_string()));

        assert!(retired.is_some());
        assert!(!source.is_current(stream_token));
        assert!(source.is_current(source.token()));
        assert!(source.path().is_some());
    }

    /// A stand-in for a custom `IStream` whose `Release` reenters the handler.
    /// The retired reference is dropped only after `clear_all` has returned and
    /// the owning `RefCell` borrow is gone, so the reentrant borrow succeeds.
    struct ReentrantStream {
        source: Weak<RefCell<DeferredSource<ReentrantStream>>>,
        reentered: Rc<Cell<bool>>,
    }

    impl Drop for ReentrantStream {
        fn drop(&mut self) {
            let Some(owner) = self.source.upgrade() else {
                return;
            };
            let retired = {
                let Ok(mut source) = owner.try_borrow_mut() else {
                    return;
                };
                source.clear_all()
            };
            // Any stream still pending in the source is released here, after
            // the `RefCell` borrow from `clear_all` has ended.
            drop(retired);
            self.reentered.set(true);
        }
    }

    #[test]
    fn replacing_a_pending_stream_returns_the_retired_reference_for_release() {
        let source = Rc::new(RefCell::new(DeferredSource::<ReentrantStream>::default()));
        let reentered = Rc::new(Cell::new(false));
        let stream = ReentrantStream {
            source: Rc::downgrade(&source),
            reentered: reentered.clone(),
        };
        assert!(source.borrow_mut().initialize_stream(stream).is_none());

        let replacement = ReentrantStream {
            source: Rc::downgrade(&source),
            reentered: reentered.clone(),
        };
        let retired = source.borrow_mut().initialize_stream(replacement);
        assert!(retired.is_some());
        assert!(
            !reentered.get(),
            "the retired reference must not drop inside the RefCell borrow"
        );

        drop(retired);
        assert!(reentered.get());
    }

    #[test]
    fn path_initialization_returns_the_retired_stream_reference_for_release() {
        let source = Rc::new(RefCell::new(DeferredSource::<ReentrantStream>::default()));
        let reentered = Rc::new(Cell::new(false));
        let stream = ReentrantStream {
            source: Rc::downgrade(&source),
            reentered: reentered.clone(),
        };
        assert!(source.borrow_mut().initialize_stream(stream).is_none());

        let retired = source
            .borrow_mut()
            .initialize_path(PathBuf::from("scan/model.obj"), Some("obj".to_string()));
        assert!(retired.is_some());
        assert!(!reentered.get());

        drop(retired);
        assert!(reentered.get());
    }

    #[test]
    fn retired_stream_release_runs_after_the_handler_refcell_borrow_ends() {
        let source = Rc::new(RefCell::new(DeferredSource::<ReentrantStream>::default()));
        let reentered = Rc::new(Cell::new(false));
        let stream = ReentrantStream {
            source: Rc::downgrade(&source),
            reentered: reentered.clone(),
        };

        let retired = source.borrow_mut().initialize_stream(stream);
        assert!(retired.is_none());
        assert!(!reentered.get());

        let retired = source.borrow_mut().clear_all();
        assert!(retired.is_some());
        assert!(
            !reentered.get(),
            "Release must not run while the handler RefCell is borrowed"
        );

        drop(retired);
        assert!(
            reentered.get(),
            "Release reentered after the handler RefCell borrow ended"
        );
    }
}
