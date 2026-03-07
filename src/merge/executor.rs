use crate::error::{Error, Result};
use crate::merge::{CleanupReport, DeltaMergePolicy, MergeCleanup, MergeTrigger};
use crate::metrics::MergeMetrics;
use crate::state::{MergeProgressState, MetadataManager, SnapshotMetaState};
use async_trait::async_trait;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::task::JoinHandle;

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeExecution {
    pub checkpoint_size_bytes: u64,
    pub checkpoint_path: Option<String>,
}

#[async_trait]
pub trait MergeBackend: Send + Sync {
    async fn execute_merge(&self, snapshot_state: &SnapshotMetaState) -> Result<MergeExecution>;
}

#[derive(Debug, Default)]
pub struct MetadataOnlyMergeBackend;

#[async_trait]
impl MergeBackend for MetadataOnlyMergeBackend {
    async fn execute_merge(&self, snapshot_state: &SnapshotMetaState) -> Result<MergeExecution> {
        Ok(MergeExecution {
            checkpoint_size_bytes: snapshot_state.total_delta_bytes,
            checkpoint_path: snapshot_state
                .last_checkpoint
                .as_ref()
                .and_then(|cp| cp.checkpoint_path.clone()),
        })
    }
}

fn merge_error_code(err: &Error) -> String {
    match err {
        Error::Snapshot(msg) => msg
            .split(':')
            .next()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| crate::merge::MERGE_ERR_UNKNOWN.to_string()),
        _ => crate::merge::MERGE_ERR_UNKNOWN.to_string(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeTaskResult {
    pub merged: bool,
    pub trigger: Option<MergeTrigger>,
    pub retries: u8,
    pub checkpoint_size_bytes: u64,
    pub cleanup_report: CleanupReport,
}

pub struct MergeTaskHandle {
    pub trigger: MergeTrigger,
    pub handle: JoinHandle<Result<MergeTaskResult>>,
}

/// Run async merge execution with retry, persistence, and metrics.
pub struct MergeExecutor {
    metadata: Arc<MetadataManager>,
    backend: Arc<dyn MergeBackend>,
    cleanup: MergeCleanup,
    policy: DeltaMergePolicy,
    metrics: MergeMetrics,
    max_retries: u8,
}

impl MergeExecutor {
    pub fn new(
        metadata: Arc<MetadataManager>,
        policy: DeltaMergePolicy,
        metrics: MergeMetrics,
        cleanup: MergeCleanup,
    ) -> Self {
        Self {
            metadata,
            backend: Arc::new(MetadataOnlyMergeBackend),
            cleanup,
            policy,
            metrics,
            max_retries: 3,
        }
    }

    pub fn with_backend(mut self, backend: Arc<dyn MergeBackend>) -> Self {
        self.backend = backend;
        self
    }

    pub fn with_max_retries(mut self, max_retries: u8) -> Self {
        self.max_retries = max_retries.max(1);
        self
    }

    pub async fn should_spawn(&self) -> Result<Option<MergeTrigger>> {
        let state = self.metadata.get_snapshot_state().await;
        Ok(self.policy.evaluate(&state, now_secs()))
    }

    pub async fn spawn_if_needed(&self) -> Result<Option<MergeTaskHandle>> {
        let Some(trigger) = self.should_spawn().await? else {
            return Ok(None);
        };

        let metadata = self.metadata.clone();
        let backend = self.backend.clone();
        let cleanup = self.cleanup.clone();
        let metrics = self.metrics.clone();
        let max_retries = self.max_retries;

        let handle = tokio::spawn(async move {
            Self::run_merge_loop(metadata, backend, cleanup, metrics, trigger, max_retries).await
        });

        Ok(Some(MergeTaskHandle { trigger, handle }))
    }

    async fn run_merge_loop(
        metadata: Arc<MetadataManager>,
        backend: Arc<dyn MergeBackend>,
        cleanup: MergeCleanup,
        metrics: MergeMetrics,
        trigger: MergeTrigger,
        max_retries: u8,
    ) -> Result<MergeTaskResult> {
        let mut retries = 0u8;
        let started = now_secs();

        loop {
            retries = retries.saturating_add(1);
            metadata
                .save_merge_progress_state(MergeProgressState::started(
                    trigger.as_str().to_string(),
                    retries,
                    max_retries,
                    started,
                ))
                .await?;

            let snapshot_state = metadata.get_snapshot_state().await;
            match backend.execute_merge(&snapshot_state).await {
                Ok(exec) => {
                    let merged_state = Self::build_merged_snapshot_state(
                        snapshot_state,
                        started,
                        &exec.checkpoint_path,
                    );
                    let progress = MergeProgressState::succeeded(
                        trigger.as_str().to_string(),
                        retries,
                        max_retries,
                        started,
                    );

                    metadata
                        .save_snapshot_and_merge_state(merged_state.clone(), progress)
                        .await?;

                    let cleanup_report = cleanup.run(&merged_state, Some(started)).await?;
                    metrics.record_merge_success(
                        trigger,
                        retries,
                        exec.checkpoint_size_bytes,
                        started,
                    );

                    return Ok(MergeTaskResult {
                        merged: true,
                        trigger: Some(trigger),
                        retries,
                        checkpoint_size_bytes: exec.checkpoint_size_bytes,
                        cleanup_report,
                    });
                }
                Err(err) => {
                    let error_code = merge_error_code(&err);
                    if retries >= max_retries {
                        metadata
                            .save_merge_progress_state(MergeProgressState::failed(
                                trigger.as_str().to_string(),
                                retries,
                                max_retries,
                                started,
                                err.to_string(),
                            ))
                            .await?;

                        metrics.record_merge_failure(trigger, retries, started, &error_code);
                        return Err(Error::Snapshot(format!(
                            "{}: merge failed after {} retries: {}",
                            error_code, retries, err
                        )));
                    }
                }
            }
        }
    }

    fn build_merged_snapshot_state(
        mut state: SnapshotMetaState,
        now: u64,
        checkpoint_path: &Option<String>,
    ) -> SnapshotMetaState {
        let end_index = state.delta_chain.last().map(|d| d.end_index);

        if let Some(cp) = state.last_checkpoint.as_mut() {
            cp.sequence_number = cp.sequence_number.saturating_add(1);
            cp.created_at = now;
            cp.checkpoint_path = checkpoint_path
                .clone()
                .or_else(|| Some(format!("target/checkpoints/checkpoint_{}", now)));
            if let Some(end) = end_index {
                cp.checkpoint_index = end;
            }
        }

        state.delta_chain.clear();
        state.total_delta_bytes = 0;
        state
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::merge::MergeCleanupConfig;
    use crate::merge::MERGE_ERR_INJECTED_FAILURE;
    use crate::state::{CheckpointMetadata, DeltaInfo};
    use surrealkv::TreeBuilder;
    use tempfile::TempDir;
    use tokio::sync::Mutex;

    struct FailThenOkBackend {
        remaining_failures: Arc<Mutex<u8>>,
    }

    #[async_trait]
    impl MergeBackend for FailThenOkBackend {
        async fn execute_merge(
            &self,
            _snapshot_state: &SnapshotMetaState,
        ) -> Result<MergeExecution> {
            let mut guard = self.remaining_failures.lock().await;
            if *guard > 0 {
                *guard -= 1;
                return Err(Error::Snapshot(format!(
                    "{}: injected failure",
                    MERGE_ERR_INJECTED_FAILURE
                )));
            }
            Ok(MergeExecution {
                checkpoint_size_bytes: 4096,
                checkpoint_path: Some("target/checkpoints/checkpoint_test".to_string()),
            })
        }
    }

    #[tokio::test]
    async fn test_executor_retry_until_success() {
        let base = TempDir::new().unwrap();
        let tree = Arc::new(
            TreeBuilder::new()
                .with_path(base.path().join("tree"))
                .build()
                .unwrap(),
        );

        let metadata = Arc::new(MetadataManager::new(tree));
        let mut state = SnapshotMetaState::new();
        state.last_checkpoint = Some(CheckpointMetadata::new(10, 2, 8, 1));
        for i in 0..5 {
            state
                .delta_chain
                .push(DeltaInfo::new(i * 10, i * 10 + 9, 10, 2));
        }
        state.total_delta_bytes = 50;
        metadata.save_snapshot_state(state).await.unwrap();

        let backend = Arc::new(FailThenOkBackend {
            remaining_failures: Arc::new(Mutex::new(1)),
        });

        let executor = MergeExecutor::new(
            metadata.clone(),
            DeltaMergePolicy::default(),
            MergeMetrics::new("1"),
            MergeCleanup::new(MergeCleanupConfig::default()),
        )
        .with_backend(backend)
        .with_max_retries(3);

        let handle = executor.spawn_if_needed().await.unwrap().unwrap();
        let out = handle.handle.await.unwrap().unwrap();

        assert!(out.merged);
        assert_eq!(out.retries, 2);

        let final_state = metadata.get_snapshot_state().await;
        assert!(final_state.delta_chain.is_empty());
        assert_eq!(final_state.total_delta_bytes, 0);
    }
}
