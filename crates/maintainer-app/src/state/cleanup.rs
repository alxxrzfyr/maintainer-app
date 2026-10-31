use crate::ui::cleanup::CleanupPhase;

/// State of the cleanup workflow page.
#[derive(Debug, Clone, Default)]
pub struct CleanupState {
    /// Current phase of the cleanup workflow.
    pub phase: CleanupPhase,
    /// Number of selected items (for enabling/disabling execute button).
    pub selected_count: usize,
    /// Total reclaimable bytes from selected items.
    pub selected_bytes: u64,
    /// Whether there are items requiring permanent deletion in the selection.
    pub has_permanent_items: bool,
}

/// Intents that drive cleanup state transitions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CleanupIntent {
    /// Start scanning for cleanup targets.
    StartScan,
    /// Update scan progress.
    ScanProgress { message: String },
    /// Scan completed with items found.
    ScanCompleted { item_count: usize, total_bytes: u64 },
    /// Scan failed.
    ScanFailed { message: String },
    /// Update selected item count.
    SelectionChanged {
        selected_count: usize,
        selected_bytes: u64,
        has_permanent: bool,
    },
    /// Start executing cleanup.
    StartExecution,
    /// Update execution progress.
    ExecutionProgress { message: String },
    /// Execution completed.
    ExecutionCompleted { summary: String },
    /// Reset to idle state.
    Reset,
}

impl CleanupState {
    /// Apply a cleanup intent and update state.
    pub fn reduce(&mut self, intent: CleanupIntent) -> bool {
        match intent {
            CleanupIntent::StartScan => {
                self.phase = CleanupPhase::Scanning {
                    message: "Starting scan…".into(),
                };
                self.selected_count = 0;
                self.selected_bytes = 0;
                self.has_permanent_items = false;
                true
            }
            CleanupIntent::ScanProgress { message } => {
                self.phase = CleanupPhase::Scanning { message };
                true
            }
            CleanupIntent::ScanCompleted {
                item_count,
                total_bytes,
            } => {
                self.phase = CleanupPhase::Reviewing {
                    item_count,
                    total_bytes,
                };
                true
            }
            CleanupIntent::ScanFailed { message } => {
                self.phase = CleanupPhase::Completed {
                    summary: format!("Scan failed: {message}"),
                };
                true
            }
            CleanupIntent::SelectionChanged {
                selected_count,
                selected_bytes,
                has_permanent,
            } => {
                self.selected_count = selected_count;
                self.selected_bytes = selected_bytes;
                self.has_permanent_items = has_permanent;
                true
            }
            CleanupIntent::StartExecution => {
                self.phase = CleanupPhase::Executing {
                    message: "Starting cleanup…".into(),
                };
                true
            }
            CleanupIntent::ExecutionProgress { message } => {
                self.phase = CleanupPhase::Executing { message };
                true
            }
            CleanupIntent::ExecutionCompleted { summary } => {
                self.phase = CleanupPhase::Completed { summary };
                self.selected_count = 0;
                self.selected_bytes = 0;
                true
            }
            CleanupIntent::Reset => {
                *self = Self::default();
                true
            }
        }
    }
}

#[cfg(test)]
mod cleanup_tests {
    use super::*;

    #[test]
    fn cleanup_state_default_is_idle() {
        let state = CleanupState::default();
        assert_eq!(state.phase, CleanupPhase::Idle);
        assert_eq!(state.selected_count, 0);
    }

    #[test]
    fn cleanup_start_scan_transitions_to_scanning() {
        let mut state = CleanupState::default();
        state.reduce(CleanupIntent::StartScan);
        assert!(matches!(state.phase, CleanupPhase::Scanning { .. }));
    }

    #[test]
    fn cleanup_scan_completed_transitions_to_reviewing() {
        let mut state = CleanupState::default();
        state.reduce(CleanupIntent::StartScan);
        state.reduce(CleanupIntent::ScanCompleted {
            item_count: 5,
            total_bytes: 1024,
        });
        assert_eq!(
            state.phase,
            CleanupPhase::Reviewing {
                item_count: 5,
                total_bytes: 1024,
            }
        );
    }

    #[test]
    fn cleanup_selection_updates() {
        let mut state = CleanupState::default();
        state.reduce(CleanupIntent::SelectionChanged {
            selected_count: 3,
            selected_bytes: 512,
            has_permanent: true,
        });
        assert_eq!(state.selected_count, 3);
        assert_eq!(state.selected_bytes, 512);
        assert!(state.has_permanent_items);
    }

    #[test]
    fn cleanup_execution_flow() {
        let mut state = CleanupState::default();
        state.reduce(CleanupIntent::StartExecution);
        assert!(matches!(state.phase, CleanupPhase::Executing { .. }));

        state.reduce(CleanupIntent::ExecutionCompleted {
            summary: "Recovered 1 MiB".into(),
        });
        assert!(matches!(state.phase, CleanupPhase::Completed { .. }));
        assert_eq!(state.selected_count, 0);
    }

    #[test]
    fn cleanup_reset_returns_to_default() {
        let mut state = CleanupState::default();
        state.reduce(CleanupIntent::StartScan);
        state.reduce(CleanupIntent::ScanCompleted {
            item_count: 10,
            total_bytes: 2048,
        });
        state.reduce(CleanupIntent::Reset);
        assert_eq!(state.phase, CleanupPhase::Idle);
    }

    #[test]
    fn cleanup_scan_failed_shows_error() {
        let mut state = CleanupState::default();
        state.reduce(CleanupIntent::StartScan);
        state.reduce(CleanupIntent::ScanFailed {
            message: "permission denied".into(),
        });
        match &state.phase {
            CleanupPhase::Completed { summary } => {
                assert!(summary.contains("permission denied"));
            }
            other => panic!("expected Completed, got {other:?}"),
        }
    }
}
