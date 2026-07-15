use std::path::{Path, PathBuf};

#[derive(Debug)]
pub(crate) struct DeferredSource<S> {
    pending_stream: Option<S>,
    path: Option<PathBuf>,
    extension: Option<String>,
}

impl<S> Default for DeferredSource<S> {
    fn default() -> Self {
        Self {
            pending_stream: None,
            path: None,
            extension: None,
        }
    }
}

impl<S> DeferredSource<S> {
    pub(crate) fn initialize_stream(&mut self, stream: S) {
        self.pending_stream = Some(stream);
        self.path = None;
        self.extension = None;
    }

    pub(crate) fn initialize_path(&mut self, path: PathBuf, extension: Option<String>) {
        self.pending_stream = None;
        self.path = Some(path);
        self.extension = extension;
    }

    pub(crate) fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub(crate) fn extension(&self) -> Option<&str> {
        self.extension.as_deref()
    }

    pub(crate) fn consume_pending_stream<R>(
        &mut self,
        f: impl FnOnce(S, Option<&str>) -> R,
    ) -> Option<R> {
        let stream = self.pending_stream.take()?;
        Some(f(stream, self.extension()))
    }

    pub(crate) fn clear_all(&mut self) {
        self.pending_stream = None;
        self.path = None;
        self.extension = None;
    }

    #[cfg(test)]
    pub(crate) fn has_pending_stream(&self) -> bool {
        self.pending_stream.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::DeferredSource;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn initialize_stream_stages_lazy_source_without_path_metadata() {
        let mut source = DeferredSource::<u8>::default();
        source.initialize_stream(7_u8);

        assert!(source.has_pending_stream());
        assert!(source.path().is_none());
        assert!(source.extension().is_none());
    }

    #[test]
    fn initialize_path_clears_pending_stream_and_sets_extension() {
        let mut source = DeferredSource::<u8>::default();
        source.initialize_stream(7_u8);
        source.initialize_path(PathBuf::from("scan/model.obj"), Some("obj".to_string()));

        assert!(!source.has_pending_stream());
        assert_eq!(source.path(), Some(Path::new("scan/model.obj")));
        assert_eq!(source.extension(), Some("obj"));
    }

    #[test]
    fn pending_stream_is_consumed_only_by_explicit_lazy_load() {
        let mut source = DeferredSource::<u8>::default();
        let call_count = Arc::new(AtomicUsize::new(0));
        source.initialize_stream(11_u8);

        assert_eq!(call_count.load(Ordering::SeqCst), 0);

        let outcome = source.consume_pending_stream({
            let call_count = call_count.clone();
            move |stream, extension| {
                call_count.fetch_add(1, Ordering::SeqCst);
                (stream, extension.map(str::to_owned))
            }
        });

        assert_eq!(outcome, Some((11_u8, None)));
        assert_eq!(call_count.load(Ordering::SeqCst), 1);
        assert!(!source.has_pending_stream());
    }

    #[test]
    fn clear_all_drops_pending_stream_and_path_state() {
        let mut source = DeferredSource::<u8>::default();
        source.initialize_path(PathBuf::from("scan/model.obj"), Some("obj".to_string()));
        source.clear_all();

        assert!(!source.has_pending_stream());
        assert!(source.path().is_none());
        assert!(source.extension().is_none());
    }
}
