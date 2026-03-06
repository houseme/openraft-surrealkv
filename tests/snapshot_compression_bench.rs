use openraft_surrealkv::snapshot::{CheckpointMetadata, SnapshotCompressor};
use std::time::Instant;
use tempfile::TempDir;
use tokio::fs;

async fn run_compression_case(bytes: usize) -> anyhow::Result<(u64, std::time::Duration)> {
    let tmp = TempDir::new()?;
    let base = tmp.path().join("ckpt");
    fs::create_dir_all(&base).await?;

    let meta = CheckpointMetadata::new(100, 5, 0, base.clone());
    let ckpt = base.join(meta.dir_name());
    fs::create_dir_all(&ckpt).await?;

    // Write deterministic payload so runs are comparable.
    let payload = vec![b'x'; bytes];
    fs::write(ckpt.join("payload.bin"), payload).await?;

    let out = tmp.path().join("snapshot.tar.zst");
    let compressor = SnapshotCompressor::new();

    let begin = Instant::now();
    let compressed_size = compressor.compress(&meta, out).await?;
    let elapsed = begin.elapsed();

    Ok((compressed_size, elapsed))
}

#[tokio::test]
async fn bench_snapshot_compression_10mb() -> anyhow::Result<()> {
    let input_size = 10 * 1024 * 1024;
    let (compressed_size, elapsed) = run_compression_case(input_size).await?;

    eprintln!(
        "[bench-10mb] input={} compressed={} ratio={:.2}x elapsed_ms={}",
        input_size,
        compressed_size,
        input_size as f64 / compressed_size as f64,
        elapsed.as_millis()
    );

    assert!(compressed_size > 0);
    Ok(())
}

#[tokio::test]
async fn bench_snapshot_compression_100mb() -> anyhow::Result<()> {
    let input_size = 100 * 1024 * 1024;
    let (compressed_size, elapsed) = run_compression_case(input_size).await?;

    eprintln!(
        "[bench-100mb] input={} compressed={} ratio={:.2}x elapsed_ms={}",
        input_size,
        compressed_size,
        input_size as f64 / compressed_size as f64,
        elapsed.as_millis()
    );

    assert!(compressed_size > 0);
    Ok(())
}

#[tokio::test]
async fn bench_snapshot_compression_300mb() -> anyhow::Result<()> {
    let input_size = 300 * 1024 * 1024;
    let (compressed_size, elapsed) = run_compression_case(input_size).await?;

    eprintln!(
        "[bench-300mb] input={} compressed={} ratio={:.2}x elapsed_ms={}",
        input_size,
        compressed_size,
        input_size as f64 / compressed_size as f64,
        elapsed.as_millis()
    );

    // Target: <10s for 300MB.
    eprintln!(
        "[bench-300mb] target: <10000ms, actual: {}ms",
        elapsed.as_millis()
    );

    assert!(compressed_size > 0);
    Ok(())
}
