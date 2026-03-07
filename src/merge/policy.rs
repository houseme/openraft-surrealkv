use crate::state::SnapshotMetaState;

/// Define merge policy configuration with three-dimensional thresholds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeltaMergePolicy {
    /// Maximum number of deltas in chain before triggering merge.
    pub max_chain_length: usize,
    /// Maximum cumulative size of deltas in bytes (300MB default).
    pub max_delta_bytes: u64,
    /// Time window for full checkpoint (24h default).
    pub checkpoint_interval_secs: u64,
}

impl DeltaMergePolicy {
    pub fn new() -> Self {
        Self {
            max_chain_length: 5,
            max_delta_bytes: 300 * 1024 * 1024,
            checkpoint_interval_secs: 24 * 60 * 60,
        }
    }

    /// Evaluate merge trigger using strict priority.
    /// Chain length > cumulative bytes > checkpoint time window.
    pub fn evaluate(
        &self,
        snapshot_state: &SnapshotMetaState,
        now_secs: u64,
    ) -> Option<MergeTrigger> {
        let decision = MergeDecision::from_state(self, snapshot_state, now_secs);
        decision.trigger
    }

    /// Return whether the current state should trigger merge.
    pub fn should_merge(&self, snapshot_state: &SnapshotMetaState, now_secs: u64) -> bool {
        self.evaluate(snapshot_state, now_secs).is_some()
    }
}

impl Default for DeltaMergePolicy {
    fn default() -> Self {
        Self::new()
    }
}

/// Define merge trigger reasons ordered by policy priority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeTrigger {
    ChainTooLong,
    SizeTooLarge,
    TimeWindowExpired,
}

impl MergeTrigger {
    pub fn as_str(self) -> &'static str {
        match self {
            MergeTrigger::ChainTooLong => "chain_too_long",
            MergeTrigger::SizeTooLarge => "size_too_large",
            MergeTrigger::TimeWindowExpired => "time_window_expired",
        }
    }
}

/// Define rich merge decision output for tests and metrics labeling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeDecision {
    pub trigger: Option<MergeTrigger>,
    pub chain_length: usize,
    pub cumulative_bytes: u64,
}

impl MergeDecision {
    pub fn from_state(
        policy: &DeltaMergePolicy,
        snapshot_state: &SnapshotMetaState,
        now_secs: u64,
    ) -> Self {
        let chain_length = snapshot_state.delta_chain.len();
        let cumulative_bytes = snapshot_state.total_delta_bytes;

        let trigger = if chain_length >= policy.max_chain_length {
            Some(MergeTrigger::ChainTooLong)
        } else if cumulative_bytes >= policy.max_delta_bytes {
            Some(MergeTrigger::SizeTooLarge)
        } else {
            snapshot_state.last_checkpoint.as_ref().and_then(|cp| {
                let age = now_secs.saturating_sub(cp.created_at);
                if age >= policy.checkpoint_interval_secs {
                    Some(MergeTrigger::TimeWindowExpired)
                } else {
                    None
                }
            })
        };

        Self {
            trigger,
            chain_length,
            cumulative_bytes,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{CheckpointMetadata, DeltaInfo};

    #[test]
    fn test_priority_chain_over_size_over_time() {
        let mut state = SnapshotMetaState::new();
        state.last_checkpoint = Some(CheckpointMetadata::new(10, 2, 3, 1));
        state.total_delta_bytes = 400 * 1024 * 1024;
        for i in 0..5 {
            state
                .delta_chain
                .push(DeltaInfo::new(i * 10, i * 10 + 9, 1024, 2));
        }

        let p = DeltaMergePolicy::default();
        let trigger = p.evaluate(&state, 24 * 60 * 60 + 10);
        assert_eq!(trigger, Some(MergeTrigger::ChainTooLong));
    }

    #[test]
    fn test_size_trigger_when_chain_below_threshold() {
        let mut state = SnapshotMetaState::new();
        state.total_delta_bytes = 300 * 1024 * 1024;

        let p = DeltaMergePolicy::default();
        let trigger = p.evaluate(&state, 1);
        assert_eq!(trigger, Some(MergeTrigger::SizeTooLarge));
    }

    #[test]
    fn test_time_trigger_when_chain_and_size_are_small() {
        let mut state = SnapshotMetaState::new();
        state.last_checkpoint = Some(CheckpointMetadata::new(8, 1, 2, 100));

        let p = DeltaMergePolicy::default();
        let trigger = p.evaluate(&state, 100 + 24 * 60 * 60);
        assert_eq!(trigger, Some(MergeTrigger::TimeWindowExpired));
    }

    #[test]
    fn test_no_trigger() {
        let mut state = SnapshotMetaState::new();
        state.last_checkpoint = Some(CheckpointMetadata::new(8, 1, 2, 100));
        state.total_delta_bytes = 1024;

        let p = DeltaMergePolicy::default();
        let trigger = p.evaluate(&state, 100 + 10);
        assert_eq!(trigger, None);
    }
}
