//! HTTP API 相关的 Prometheus 指标

use metrics::{counter, histogram};
use std::time::Instant;

/// HTTP API 指标记录器
pub struct HttpMetrics {
    node_id: u64,
}

impl HttpMetrics {
    pub fn new(node_id: u64) -> Self {
        Self { node_id }
    }

    /// 记录 HTTP 请求
    pub fn record_request(&self, method: &str, endpoint: &str, status: u16, latency_ms: u64) {
        counter!(
            "http_requests_total",
            "node_id" => self.node_id.to_string(),
            "method" => method.to_string(),
            "endpoint" => endpoint.to_string(),
            "status" => status.to_string(),
        )
        .increment(1);

        histogram!(
            "http_request_duration_ms",
            "node_id" => self.node_id.to_string(),
            "method" => method.to_string(),
            "endpoint" => endpoint.to_string(),
            "status" => status.to_string(),
        )
        .record(latency_ms as f64);
    }

    /// 记录 GET /kv 请求
    pub fn record_get_kv(&self, found: bool, latency_ms: u64) {
        let status = if found { 200 } else { 404 };
        self.record_request("GET", "/kv/:key", status, latency_ms);
    }

    /// 记录 POST /kv 请求
    pub fn record_put_kv(&self, success: bool, latency_ms: u64) {
        let status = if success { 200 } else { 500 };
        self.record_request("POST", "/kv/:key", status, latency_ms);
    }

    /// 记录 DELETE /kv 请求
    pub fn record_delete_kv(&self, success: bool, latency_ms: u64) {
        let status = if success { 200 } else { 500 };
        self.record_request("DELETE", "/kv/:key", status, latency_ms);
    }

    /// 记录 GET /health 请求
    pub fn record_health_check(&self, latency_ms: u64) {
        self.record_request("GET", "/health", 200, latency_ms);
    }

    /// 记录 GET /status 请求
    pub fn record_status(&self, latency_ms: u64) {
        self.record_request("GET", "/status", 200, latency_ms);
    }
}

/// 请求计时器
pub struct RequestTimer {
    start: Instant,
}

impl RequestTimer {
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
        }
    }

    pub fn elapsed_ms(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }
}

impl Default for RequestTimer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_http_metrics_creation() {
        let metrics = HttpMetrics::new(1);
        metrics.record_get_kv(true, 5);
        metrics.record_put_kv(true, 10);
        metrics.record_delete_kv(true, 8);
        metrics.record_health_check(1);
    }

    #[test]
    fn test_request_timer() {
        let timer = RequestTimer::new();
        std::thread::sleep(std::time::Duration::from_millis(10));
        assert!(timer.elapsed_ms() >= 10);
    }
}
