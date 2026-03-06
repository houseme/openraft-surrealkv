use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use std::sync::OnceLock;

static PROMETHEUS_HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();

/// Initialize global Prometheus recorder once.
pub fn init_prometheus_recorder() -> anyhow::Result<()> {
    if PROMETHEUS_HANDLE.get().is_some() {
        return Ok(());
    }

    let recorder = PrometheusBuilder::new().build_recorder();
    let handle = recorder.handle();
    metrics::set_global_recorder(recorder)
        .map_err(|e| anyhow::anyhow!("failed to install metrics recorder: {}", e))?;

    let _ = PROMETHEUS_HANDLE.set(handle);
    Ok(())
}

/// Render metrics in Prometheus text format.
pub fn render_metrics() -> String {
    match PROMETHEUS_HANDLE.get() {
        Some(handle) => handle.render(),
        None => "# prometheus recorder not initialized\n".to_string(),
    }
}
