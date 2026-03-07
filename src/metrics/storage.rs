//! Prometheus metrics related to storage

use metrics::{counter, gauge};

/// Storage metrics recorder.
pub struct StorageMetrics {
    node_id: u64,
}

impl StorageMetrics {
    pub fn new(node_id: u64) -> Self {
        Self { node_id }
    }

    /// Record total key count.
    pub fn record_keys_total(&self, count: u64) {
        gauge!("storage_keys_total", "node_id" => self.node_id.to_string()).set(count as f64);
    }

    /// Record storage size (bytes).
    pub fn record_size_bytes(&self, size: u64) {
        gauge!("storage_size_bytes", "node_id" => self.node_id.to_string()).set(size as f64);
    }

    /// Record read operations.
    pub fn record_read(&self, success: bool) {
        counter!("storage_reads_total", "node_id" => self.node_id.to_string(), "status" => if success { "success" } else { "failed" })
            .increment(1);
    }

    /// Record write operations.
    pub fn record_write(&self, success: bool) {
        counter!("storage_writes_total", "node_id" => self.node_id.to_string(), "status" => if success { "success" } else { "failed" })
            .increment(1);
    }

    /// Record delete operations.
    pub fn record_delete(&self, success: bool) {
        counter!("storage_deletes_total", "node_id" => self.node_id.to_string(), "status" => if success { "success" } else { "failed" })
            .increment(1);
    }

    /// Record snapshot creation.
    pub fn record_snapshot_created(&self) {
        counter!("storage_snapshots_created_total", "node_id" => self.node_id.to_string())
            .increment(1);
    }

    /// Record snapshot restore.
    pub fn record_snapshot_restored(&self) {
        counter!("storage_snapshots_restored_total", "node_id" => self.node_id.to_string())
            .increment(1);
    }

    /// Record log entry count.
    pub fn record_log_entries(&self, count: u64) {
        gauge!("storage_log_entries", "node_id" => self.node_id.to_string()).set(count as f64);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_storage_metrics_creation() {
        let metrics = StorageMetrics::new(1);
        metrics.record_keys_total(100);
        metrics.record_size_bytes(1024 * 1024);
        metrics.record_read(true);
        metrics.record_write(true);
        metrics.record_snapshot_created();
    }
}
