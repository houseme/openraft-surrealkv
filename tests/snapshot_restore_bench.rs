use openraft_surrealkv::snapshot::{CheckpointMetadata, SnapshotCompressor, SnapshotRestorer};
use std::time::Instant;
use tempfile::TempDir;
use tokio::fs;

async fn run_restore_case(bytes: usize) -> anyhow::Result<(u64, std::time::Duration)> {
    let tmp = TempDir::new()?;
    let base = tmp.path().join("ckpt");
    fs::create_dir_all(&base).await?;

    let meta = CheckpointMetadata::new(100, 5, 0, base.clone());
    let ckpt = base.join(meta.dir_name());
    fs::create_dir_all(&ckpt).await?;

    // Write deterministic payload.
    let payload = vec![b'x'; bytes];
    fs::write(ckpt.join("payload.bin"), payload).await?;

    let out = tmp.path().join("snapshot.tar.zst");
    let compressor = SnapshotCompressor::new();
    let compressed_size = compressor.compress(&meta, out.clone()).await?;

    let compressed = fs::read(out).await?;
    let restored_dir = tmp.path().join("restored");

    let begin = Instant::now();
    SnapshotRestorer::restore(&compressed, restored_dir.clone(), &meta).await?;
    let elapsed = begin.elapsed();

    Ok((compressed_size, elapsed))
}

#[tokio::test]
async fn bench_snapshot_restore_10mb() -> anyhow::Result<()> {
    let input_size = 10 * 1024 * 1024;
    let (compressed_size, elapsed) = run_restore_case(input_size).await?;

    eprintln!(
        "[bench-restore-10mb] input={} compressed={} elapsed_ms={}",
        input_size,
        compressed_size,
        elapsed.as_millis()
    );

    assert!(elapsed.as_secs() < 10);
    Ok(())
}

#[tokio::test]
async fn bench_snapshot_restore_100mb() -> anyhow::Result<()> {
    let input_size = 100 * 1024 * 1024;
    let (compressed_size, elapsed) = run_restore_case(input_size).await?;

    eprintln!(
        "[bench-restore-100mb] input={} compressed={} elapsed_ms={}",
        input_size,
        compressed_size,
        elapsed.as_millis()
    );

    assert!(elapsed.as_secs() < 10);
    Ok(())
}

#[tokio::test]
async fn bench_snapshot_restore_300mb() -> anyhow::Result<()> {
    let input_size = 300 * 1024 * 1024;
    let (compressed_size, elapsed) = run_restore_case(input_size).await?;

    eprintln!(
        "[bench-restore-300mb] input={} compressed={} elapsed_ms={}",
        input_size,
        compressed_size,
        elapsed.as_millis()
    );

    // Target: <2s for 300MB decompression + restore.
    eprintln!(
        "[bench-restore-300mb] target: <2000ms, actual: {}ms",
        elapsed.as_millis()
    );

    assert!(elapsed.as_secs() < 10);
    Ok(())
}
