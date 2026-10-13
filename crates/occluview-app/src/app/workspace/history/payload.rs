//! Every retained command declares its own allocation contribution.

use std::any::Any;

use super::{MemoryContribution, TransferRecord};

pub(crate) trait HistoryPayload: Any {
    /// Separate exclusive metadata from shared immutable allocations. The
    /// journal deduplicates shared allocations across commands/checkpoints.
    fn memory_contribution(&self, declared_bytes: usize) -> MemoryContribution;
}

impl dyn HistoryPayload {
    pub(super) fn downcast_ref<T: Any>(&self) -> Option<&T> {
        (self as &dyn Any).downcast_ref()
    }

    pub(super) fn downcast_mut<T: Any>(&mut self) -> Option<&mut T> {
        (self as &mut dyn Any).downcast_mut()
    }

    pub(super) fn downcast<T: Any>(self: Box<Self>) -> Result<Box<T>, Box<dyn Any>> {
        let value: Box<dyn Any> = self;
        value.downcast()
    }
}

impl HistoryPayload for TransferRecord {
    fn memory_contribution(&self, declared_bytes: usize) -> MemoryContribution {
        // Transfers retain identities/paths, not mesh buffers. Their preflight
        // supplies the metadata estimate before either scene is changed.
        MemoryContribution::exclusive(declared_bytes)
    }
}

#[cfg(test)]
impl HistoryPayload for String {
    fn memory_contribution(&self, declared_bytes: usize) -> MemoryContribution {
        MemoryContribution::exclusive(declared_bytes)
    }
}

#[cfg(test)]
impl HistoryPayload for &'static str {
    fn memory_contribution(&self, declared_bytes: usize) -> MemoryContribution {
        MemoryContribution::exclusive(declared_bytes)
    }
}
