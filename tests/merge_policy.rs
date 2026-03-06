use openraft_surrealkv::merge::{DeltaMergePolicy, MergeTrigger};
use openraft_surrealkv::state::{CheckpointMetadata, DeltaInfo, SnapshotMetaState};

#[test]
fn chain_length_trigger_has_highest_priority() {
    let mut state = SnapshotMetaState::new();
    state.last_checkpoint = Some(CheckpointMetadata::new(20, 3, 8, 1));
    state.total_delta_bytes = 500 * 1024 * 1024;

    for i in 0..5 {
        state
            .delta_chain
            .push(DeltaInfo::new(i * 10, i * 10 + 9, 10, 2));
    }

    let p = DeltaMergePolicy::default();
    assert_eq!(
        p.evaluate(&state, 24 * 60 * 60 + 100),
        Some(MergeTrigger::ChainTooLong)
    );
}

#[test]
fn size_trigger_works() {
    let mut state = SnapshotMetaState::new();
    state.total_delta_bytes = 300 * 1024 * 1024;

    let p = DeltaMergePolicy::default();
    assert_eq!(p.evaluate(&state, 100), Some(MergeTrigger::SizeTooLarge));
}

#[test]
fn time_trigger_works() {
    let mut state = SnapshotMetaState::new();
    state.last_checkpoint = Some(CheckpointMetadata::new(100, 8, 88, 1));
    state.total_delta_bytes = 1;

    let p = DeltaMergePolicy::default();
    assert_eq!(
        p.evaluate(&state, 1 + p.checkpoint_interval_secs),
        Some(MergeTrigger::TimeWindowExpired)
    );
}
