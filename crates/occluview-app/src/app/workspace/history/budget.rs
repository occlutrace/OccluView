//! History budgeting: how many commands and bytes survive.

use std::collections::HashSet;

use super::*;

impl WorkspaceHistory {
    pub(crate) fn constrain_limits(&mut self, max_count: usize, max_bytes: usize) {
        self.max_count = self.max_count.min(max_count);
        self.max_bytes = self.max_bytes.min(max_bytes);
        self.trim_to_budget();
    }
    fn is_protected(&self, id: HistoryCommandId) -> bool {
        self.checkpoints.values().any(|checkpoint| {
            checkpoint.undo_at_start.contains(&id) || checkpoint.redo_at_start.contains(&id)
        })
    }
    pub(super) fn make_room(
        &mut self,
        add_bytes: usize,
        add_count: usize,
        exclude: Option<HistoryCommandId>,
    ) -> bool {
        let Some(plan) = self.plan_room(add_bytes, add_count, exclude) else {
            return false;
        };
        self.remove_commands_with_linked_prefixes(plan.into_iter().collect());
        true
    }
    /// Compute which oldest commands must be evicted to make room without
    /// changing history. Transfer admission stores this plan on its pending
    /// entry and only applies it after the document transaction succeeds.
    pub(super) fn plan_room(
        &self,
        add_bytes: usize,
        add_count: usize,
        exclude: Option<HistoryCommandId>,
    ) -> Option<Vec<HistoryCommandId>> {
        let mut remaining_count = self
            .entries
            .len()
            .saturating_add(self.pending.len())
            .saturating_add(add_count);
        let mut remaining_bytes = self.used_bytes.saturating_add(add_bytes);
        let mut candidate_order = self.entry_order.clone();
        let mut projected_timelines = self.timelines.clone();
        let mut planned = HashSet::new();
        let mut plan = Vec::new();

        loop {
            let count = remaining_count;
            let bytes = remaining_bytes;
            if count <= self.max_count && bytes <= self.max_bytes {
                return Some(plan);
            }
            let candidate = candidate_order.iter().copied().find(|id| {
                Some(*id) != exclude && !planned.contains(id) && !self.is_protected(*id)
            })?;
            if !self.entries.contains_key(&candidate) {
                candidate_order.retain(|id| *id != candidate);
                continue;
            }

            let ids = Self::collect_linked_prefixes(
                &self.entries,
                &mut projected_timelines,
                HashSet::from([candidate]),
            );
            if ids.iter().any(|id| self.is_protected(*id)) {
                return None;
            }
            candidate_order.retain(|id| !ids.contains(id));
            for id in ids {
                if !planned.insert(id) {
                    continue;
                }
                let Some(entry) = self.entries.get(&id) else {
                    continue;
                };
                remaining_count = remaining_count.saturating_sub(1);
                remaining_bytes = remaining_bytes.saturating_sub(entry.bytes);
                plan.push(id);
            }
        }
    }
    pub(super) fn trim_to_budget(&mut self) {
        let _ = self.make_room(0, 0, None);
        // Protected checkpoints may temporarily keep the workspace above its
        // soft history cap. New records are refused until Done/Cancel releases
        // the baseline; existing snapshots are not dropped under the operator.
    }
}
