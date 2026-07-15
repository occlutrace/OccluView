use std::collections::VecDeque;

struct UndoSnapshot<T> {
    value: T,
    bytes: usize,
}

/// Memory- and count-capped undo/redo stack for edit sessions.
pub(crate) struct UndoStack<T> {
    undo: VecDeque<UndoSnapshot<T>>,
    redo: VecDeque<UndoSnapshot<T>>,
    undo_bytes: usize,
    redo_bytes: usize,
    max_count: usize,
    max_bytes: usize,
    /// Redo stack displaced by the most recent `push_undo`, kept aside until
    /// the op is known to have changed content. A real edit `commit`s it away;
    /// a content no-op `discard`s the pushed undo and RESTORES this — otherwise
    /// a no-op op would silently destroy a valid redo history.
    displaced_redo: Option<(VecDeque<UndoSnapshot<T>>, usize)>,
}

impl<T> UndoStack<T> {
    pub(crate) fn new(max_count: usize, max_bytes: usize) -> Self {
        Self {
            undo: VecDeque::new(),
            redo: VecDeque::new(),
            undo_bytes: 0,
            redo_bytes: 0,
            max_count,
            max_bytes,
            displaced_redo: None,
        }
    }

    pub(crate) fn push_undo(&mut self, value: T, bytes: usize) -> bool {
        if !self.can_store(bytes) {
            return false;
        }
        // Set the live redo aside instead of dropping it outright: the op that
        // triggered this push might turn out to be a content no-op, in which
        // case `discard_last_undo` puts the redo history back.
        self.displaced_redo = Some((
            std::mem::take(&mut self.redo),
            std::mem::replace(&mut self.redo_bytes, 0),
        ));
        Self::push_bounded(
            &mut self.undo,
            &mut self.undo_bytes,
            UndoSnapshot { value, bytes },
            self.max_count,
            self.max_bytes,
        );
        true
    }

    /// Finalize the redo-clear from the most recent `push_undo`: the op changed
    /// content, so the displaced redo history is permanently invalidated.
    pub(crate) fn commit_last_undo(&mut self) {
        self.displaced_redo = None;
    }

    pub(crate) fn peek_undo(&self) -> Option<&T> {
        self.undo.back().map(|snapshot| &snapshot.value)
    }

    /// Mutable access to the most recent undo snapshot, so a caller can stamp
    /// late-known metadata onto it (the structural-history guard fingerprint is
    /// only knowable AFTER the op that pushed the snapshot has mutated the
    /// scene). The stored byte size is unchanged by such in-place edits.
    pub(crate) fn peek_undo_mut(&mut self) -> Option<&mut T> {
        self.undo.back_mut().map(|snapshot| &mut snapshot.value)
    }

    pub(crate) fn undo(&mut self, current: T, current_bytes: usize) -> Option<T> {
        let snapshot = self.undo.pop_back()?;
        self.undo_bytes = self.undo_bytes.saturating_sub(snapshot.bytes);
        if self.can_store(current_bytes) {
            Self::push_bounded(
                &mut self.redo,
                &mut self.redo_bytes,
                UndoSnapshot {
                    value: current,
                    bytes: current_bytes,
                },
                self.max_count,
                self.max_bytes,
            );
        }
        Some(snapshot.value)
    }

    pub(crate) fn peek_redo(&self) -> Option<&T> {
        self.redo.back().map(|snapshot| &snapshot.value)
    }

    pub(crate) fn redo(&mut self, current: T, current_bytes: usize) -> Option<T> {
        let snapshot = self.redo.pop_back()?;
        self.redo_bytes = self.redo_bytes.saturating_sub(snapshot.bytes);
        if self.can_store(current_bytes) {
            Self::push_bounded(
                &mut self.undo,
                &mut self.undo_bytes,
                UndoSnapshot {
                    value: current,
                    bytes: current_bytes,
                },
                self.max_count,
                self.max_bytes,
            );
        }
        Some(snapshot.value)
    }

    /// Drop the most recent undo snapshot, and RESTORE the redo history that
    /// its `push_undo` displaced. Used when an op turns out to be a content
    /// no-op: its pre-op snapshot must not linger as a phantom undo step, and
    /// the redo stack must survive exactly as it was before the op ran.
    pub(crate) fn discard_last_undo(&mut self) {
        if let Some(snapshot) = self.undo.pop_back() {
            self.undo_bytes = self.undo_bytes.saturating_sub(snapshot.bytes);
        }
        if let Some((redo, redo_bytes)) = self.displaced_redo.take() {
            self.redo = redo;
            self.redo_bytes = redo_bytes;
        }
    }

    pub(crate) fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.undo_bytes = 0;
        self.redo_bytes = 0;
        self.displaced_redo = None;
    }

    #[cfg(test)]
    pub(crate) fn undo_len(&self) -> usize {
        self.undo.len()
    }

    #[cfg(test)]
    pub(crate) fn redo_len(&self) -> usize {
        self.redo.len()
    }

    #[cfg(test)]
    pub(crate) fn undo_bytes(&self) -> usize {
        self.undo_bytes
    }

    #[cfg(test)]
    pub(crate) fn redo_bytes(&self) -> usize {
        self.redo_bytes
    }

    fn can_store(&self, bytes: usize) -> bool {
        self.max_count > 0 && bytes <= self.max_bytes
    }

    fn push_bounded(
        stack: &mut VecDeque<UndoSnapshot<T>>,
        total_bytes: &mut usize,
        snapshot: UndoSnapshot<T>,
        max_count: usize,
        max_bytes: usize,
    ) {
        *total_bytes = total_bytes.saturating_add(snapshot.bytes);
        stack.push_back(snapshot);
        while stack.len() > max_count || *total_bytes > max_bytes {
            if let Some(removed) = stack.pop_front() {
                *total_bytes = total_bytes.saturating_sub(removed.bytes);
            } else {
                *total_bytes = 0;
                break;
            }
        }
    }
}
