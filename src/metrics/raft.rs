//! Prometheus metrics related to Raft

use metrics::{counter, gauge, histogram};

/// Raft metrics recorder.
pub struct RaftMetrics {
    node_id: u64,
}

impl RaftMetrics {
    pub fn new(node_id: u64) -> Self {
        Self { node_id }
    }

    /// Record the current term.
    pub fn record_current_term(&self, term: u64) {
        gauge!("raft_current_term", "node_id" => self.node_id.to_string()).set(term as f64);
    }

    /// Record Raft state.
    pub fn record_state(&self, state: &str) {
        gauge!("raft_state", "node_id" => self.node_id.to_string(), "state" => state.to_string())
            .set(1.0);
    }

    /// Record committed index.
    pub fn record_commit_index(&self, index: u64) {
        gauge!("raft_commit_index", "node_id" => self.node_id.to_string()).set(index as f64);
    }

    /// Record applied index.
    pub fn record_applied_index(&self, index: u64) {
        gauge!("raft_applied_index", "node_id" => self.node_id.to_string()).set(index as f64);
    }

    /// Record log length.
    pub fn record_log_length(&self, length: u64) {
        gauge!("raft_log_length", "node_id" => self.node_id.to_string()).set(length as f64);
    }

    /// Record election latency (milliseconds).
    pub fn record_election_latency_ms(&self, latency_ms: u64) {
        histogram!("raft_election_latency_ms", "node_id" => self.node_id.to_string())
            .record(latency_ms as f64);
    }

    /// Record replication latency (milliseconds).
    pub fn record_replication_latency_ms(&self, latency_ms: u64, peer: u64) {
        histogram!("raft_replication_latency_ms", "node_id" => self.node_id.to_string(), "peer" => peer.to_string())
            .record(latency_ms as f64);
    }

    /// Record successful AppendEntries RPCs.
    pub fn record_append_entries_success(&self, peer: u64) {
        counter!("raft_append_entries_total", "node_id" => self.node_id.to_string(), "peer" => peer.to_string(), "status" => "success")
            .increment(1);
    }

    /// Record failed AppendEntries RPCs.
    pub fn record_append_entries_failure(&self, peer: u64) {
        counter!("raft_append_entries_total", "node_id" => self.node_id.to_string(), "peer" => peer.to_string(), "status" => "failure")
            .increment(1);
    }

    /// Record RequestVote RPCs.
    pub fn record_vote_request(&self, granted: bool) {
        counter!("raft_vote_requests_total", "node_id" => self.node_id.to_string(), "granted" => granted.to_string())
            .increment(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_raft_metrics_creation() {
        let metrics = RaftMetrics::new(1);
        metrics.record_current_term(5);
        metrics.record_state("Leader");
        metrics.record_commit_index(100);
        metrics.record_applied_index(100);
    }
}
