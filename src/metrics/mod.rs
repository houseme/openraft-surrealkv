//! Prometheus metrics export module (Phase 5.3 + existing Phase 4 capabilities)

pub mod api;
pub mod exporter;
pub mod raft;
pub mod storage;

pub use api::{HttpMetrics, RequestTimer};
pub use exporter::{init_prometheus_recorder, render_metrics};
pub use raft::RaftMetrics;
pub use storage::StorageMetrics;

// Re-export Phase 4 merge metrics
pub use crate::merge::MergeTrigger;

/// Snapshot strict-error stages used by logs and metrics labels.
#[derive(Debug, Clone, Copy)]
pub enum SnapshotStage {
    DecodePayload,
    ValidateBase,
    ApplyDelta,
    RejectPayload,
}

impl SnapshotStage {
    pub fn as_str(self) -> &'static str {
        match self {
            SnapshotStage::DecodePayload => "decode_payload",
            SnapshotStage::ValidateBase => "validate_base",
            SnapshotStage::ApplyDelta => "apply_delta",
            SnapshotStage::RejectPayload => "reject_payload",
        }
    }

    pub fn strict_error_metric_name(self) -> &'static str {
        match self {
            SnapshotStage::DecodePayload => "snapshot_strict_error_decode_payload_total",
            SnapshotStage::ValidateBase => "snapshot_strict_error_validate_base_total",
            SnapshotStage::ApplyDelta => "snapshot_strict_error_apply_delta_total",
            SnapshotStage::RejectPayload => "snapshot_strict_error_reject_payload_total",
        }
    }
}

/// Record one strict snapshot error with stable labels for alerting and dashboards.
pub fn record_snapshot_strict_error(
    kind: &str,
    stage: SnapshotStage,
    base_index: u64,
    applied_index: u64,
) {
    metrics::counter!(
        stage.strict_error_metric_name(),
        "error_key" => kind.to_string(),
        "snapshot_stage" => stage.as_str().to_string(),
        "base_index" => base_index.to_string(),
        "applied_index" => applied_index.to_string(),
    )
    .increment(1);
}

/// Merge metrics names (Phase 4) are centralized here to avoid string drift.
pub const METRIC_MERGE_DURATION_MS: &str = "raft_snapshot_merge_duration_ms";
pub const METRIC_MERGE_SUCCESS_TOTAL: &str = "raft_snapshot_merge_success_total";
pub const METRIC_MERGE_FAILED_TOTAL: &str = "raft_snapshot_merge_failed_total";
pub const METRIC_CHECKPOINT_SIZE_BYTES: &str = "raft_snapshot_checkpoint_size_bytes";
pub const METRIC_DELTA_CHAIN_LENGTH: &str = "raft_snapshot_delta_chain_length";
pub const METRIC_DELTA_CUMULATIVE_MB: &str = "raft_snapshot_delta_cumulative_mb";

/// Merge metric recorder facade used by merge executor.
#[derive(Debug, Clone)]
pub struct MergeMetrics {
    node_id: String,
}

impl MergeMetrics {
    pub fn new(node_id: impl Into<String>) -> Self {
        Self {
            node_id: node_id.into(),
        }
    }

    pub fn record_snapshot_chain(&self, chain_len: usize, cumulative_bytes: u64) {
        metrics::gauge!(METRIC_DELTA_CHAIN_LENGTH, "node_id" => self.node_id.clone())
            .set(chain_len as f64);
        metrics::gauge!(METRIC_DELTA_CUMULATIVE_MB, "node_id" => self.node_id.clone())
            .set((cumulative_bytes as f64) / (1024.0 * 1024.0));
    }

    pub fn record_merge_success(
        &self,
        trigger: MergeTrigger,
        retries: u8,
        checkpoint_size_bytes: u64,
        started_at_secs: u64,
    ) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(started_at_secs);

        let duration_ms = now.saturating_sub(started_at_secs).saturating_mul(1000);
        metrics::histogram!(
            METRIC_MERGE_DURATION_MS,
            "node_id" => self.node_id.clone(),
            "trigger" => trigger.as_str().to_string(),
            "retries" => retries.to_string(),
        )
        .record(duration_ms as f64);
        metrics::counter!(
            METRIC_MERGE_SUCCESS_TOTAL,
            "node_id" => self.node_id.clone(),
            "trigger" => trigger.as_str().to_string(),
        )
        .increment(1);
        metrics::gauge!(METRIC_CHECKPOINT_SIZE_BYTES, "node_id" => self.node_id.clone())
            .set(checkpoint_size_bytes as f64);
    }

    pub fn record_merge_failure(
        &self,
        trigger: MergeTrigger,
        retries: u8,
        started_at_secs: u64,
        error_code: &str,
    ) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(started_at_secs);
        let duration_ms = now.saturating_sub(started_at_secs).saturating_mul(1000);

        metrics::histogram!(
            METRIC_MERGE_DURATION_MS,
            "node_id" => self.node_id.clone(),
            "trigger" => trigger.as_str().to_string(),
            "retries" => retries.to_string(),
            "result" => "failed".to_string(),
            "error_code" => error_code.to_string(),
        )
        .record(duration_ms as f64);

        metrics::counter!(
            METRIC_MERGE_FAILED_TOTAL,
            "node_id" => self.node_id.clone(),
            "trigger" => trigger.as_str().to_string(),
            "error_code" => error_code.to_string(),
        )
        .increment(1);
    }
}

/// Application-level metrics aggregator (Phase 5.3).
pub struct AppMetrics {
    pub raft: RaftMetrics,
    pub storage: StorageMetrics,
    pub api: HttpMetrics,
    pub merge: MergeMetrics,
}

impl AppMetrics {
    pub fn new(node_id: u64) -> Self {
        let node_id_str = node_id.to_string();
        Self {
            raft: RaftMetrics::new(node_id),
            storage: StorageMetrics::new(node_id),
            api: HttpMetrics::new(node_id),
            merge: MergeMetrics::new(node_id_str),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_snapshot_stage_names_stable() {
        assert_eq!(SnapshotStage::DecodePayload.as_str(), "decode_payload");
        assert_eq!(SnapshotStage::ValidateBase.as_str(), "validate_base");
        assert_eq!(SnapshotStage::ApplyDelta.as_str(), "apply_delta");
        assert_eq!(SnapshotStage::RejectPayload.as_str(), "reject_payload");
    }

    #[test]
    fn test_merge_metric_names_stable() {
        assert_eq!(METRIC_MERGE_DURATION_MS, "raft_snapshot_merge_duration_ms");
        assert_eq!(
            METRIC_MERGE_SUCCESS_TOTAL,
            "raft_snapshot_merge_success_total"
        );
        assert_eq!(
            METRIC_MERGE_FAILED_TOTAL,
            "raft_snapshot_merge_failed_total"
        );
        assert_eq!(
            METRIC_CHECKPOINT_SIZE_BYTES,
            "raft_snapshot_checkpoint_size_bytes"
        );
    }

    #[test]
    fn test_app_metrics_creation() {
        let metrics = AppMetrics::new(1);
        metrics.raft.record_current_term(1);
        metrics.storage.record_keys_total(100);
        metrics.api.record_health_check(1);
    }
}
