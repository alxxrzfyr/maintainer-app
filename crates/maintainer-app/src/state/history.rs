/// Intents for the history view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryIntent {
    /// Request deletion of a single entry by ID (triggers confirmation).
    RequestDeleteOne(String),
    /// Request deletion of multiple entries by ID (triggers confirmation).
    RequestDeleteMany(Vec<String>),
    /// Request deletion of all entries (triggers confirmation).
    RequestDeleteAll,
    /// Confirm the pending deletion.
    ConfirmDelete,
    /// Cancel the pending deletion.
    CancelDelete,
}

/// Pending deletion target for the history view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PendingHistoryDeletion {
    /// Deleting a single entry by ID.
    One(String),
    /// Deleting multiple entries by ID.
    Many(Vec<String>),
    /// Deleting all entries.
    All,
}

/// State for the history view's confirmation flow.
#[derive(Debug, Clone, Default)]
pub struct HistoryState {
    /// Pending deletion confirmation, if any.
    pub pending_deletion: Option<PendingHistoryDeletion>,
}

impl HistoryState {
    /// Apply a history intent. Returns `true` if state changed.
    pub fn reduce(&mut self, intent: HistoryIntent) -> bool {
        match intent {
            HistoryIntent::RequestDeleteOne(id) => {
                self.pending_deletion = Some(PendingHistoryDeletion::One(id));
                true
            }
            HistoryIntent::RequestDeleteMany(ids) => {
                self.pending_deletion = Some(PendingHistoryDeletion::Many(ids));
                true
            }
            HistoryIntent::RequestDeleteAll => {
                self.pending_deletion = Some(PendingHistoryDeletion::All);
                true
            }
            HistoryIntent::ConfirmDelete => self.pending_deletion.is_some(),
            HistoryIntent::CancelDelete => {
                if self.pending_deletion.is_some() {
                    self.pending_deletion = None;
                    true
                } else {
                    false
                }
            }
        }
    }
}

#[cfg(test)]
mod history_state_tests {
    use super::*;

    #[test]
    fn request_delete_one_sets_pending() {
        let mut state = HistoryState::default();
        state.reduce(HistoryIntent::RequestDeleteOne("abc".into()));
        assert_eq!(
            state.pending_deletion,
            Some(PendingHistoryDeletion::One("abc".into()))
        );
    }

    #[test]
    fn request_delete_all_sets_pending() {
        let mut state = HistoryState::default();
        state.reduce(HistoryIntent::RequestDeleteAll);
        assert_eq!(state.pending_deletion, Some(PendingHistoryDeletion::All));
    }

    #[test]
    fn cancel_clears_pending() {
        let mut state = HistoryState::default();
        state.reduce(HistoryIntent::RequestDeleteAll);
        state.reduce(HistoryIntent::CancelDelete);
        assert_eq!(state.pending_deletion, None);
    }

    #[test]
    fn confirm_without_pending_is_noop() {
        let mut state = HistoryState::default();
        let changed = state.reduce(HistoryIntent::ConfirmDelete);
        assert!(!changed);
    }

    #[test]
    fn cancel_without_pending_is_noop() {
        let mut state = HistoryState::default();
        let changed = state.reduce(HistoryIntent::CancelDelete);
        assert!(!changed);
    }
}
