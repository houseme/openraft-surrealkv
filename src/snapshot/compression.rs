//! Snapshot compression for Phase 2
//!
//! This module implements tar + zstd compression for checkpoints.
//! Target: 4-6x compression ratio, <10s for 300MB.

use super::metadata::CheckpointMetadata;
use crate::error::{Error, Result};
use std::fs::File;
use std::io;
use std::path::PathBuf;
use tokio::fs;

/// Compressor for checkpoint snapshots
pub struct SnapshotCompressor {
    zstd_level: i32,
}

impl SnapshotCompressor {
    /// Create a new compressor with default zstd level (10)
    pub fn new() -> Self {
        Self { zstd_level: 10 }
    }

    /// Set custom zstd compression level (1-22)
    pub fn with_level(mut self, level: i32) -> Self {
        self.zstd_level = level;
        self
    }

    /// Compress checkpoint directory to tar.zst format
    ///
    /// # Arguments
    /// * `meta` - Checkpoint metadata with directory path
    /// * `output_path` - Where to write the compressed snapshot
    ///
    /// # Performance targets
    /// - Compression: <10s for 300MB (zstd level 10)
    /// - Ratio: 4-6x compression
    pub async fn compress(&self, meta: &CheckpointMetadata, output_path: PathBuf) -> Result<u64> {
        let checkpoint_path = meta.checkpoint_dir.join(meta.dir_name());
        if !checkpoint_path.exists() {
            return Err(Error::Snapshot(format!(
                "checkpoint directory does not exist: {}",
                checkpoint_path.display()
            )));
        }

        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).await?;
        }

        let level = self.zstd_level;
        let checkpoint_path_cloned = checkpoint_path.clone();
        let output_path_cloned = output_path.clone();

        let compressed_size = tokio::task::spawn_blocking(move || -> io::Result<u64> {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| io::Error::other(format!("failed to build runtime: {}", e)))?;

            let tar_tmp = output_path_cloned.with_extension("tar.tmp");

            rt.block_on(async {
                let tar_file = tokio::fs::File::create(&tar_tmp).await?;
                let mut builder = tokio_tar::Builder::new(tar_file);
                builder.follow_symlinks(false);
                builder
                    .append_dir_all("checkpoint", &checkpoint_path_cloned)
                    .await?;
                builder.finish().await?;
                io::Result::Ok(())
            })?;

            let mut tar_in = File::open(&tar_tmp)?;
            let out_file = File::create(&output_path_cloned)?;
            let mut encoder = zstd::stream::write::Encoder::new(out_file, level)?;
            io::copy(&mut tar_in, &mut encoder)?;
            encoder.finish()?;

            let _ = std::fs::remove_file(&tar_tmp);

            let size = std::fs::metadata(&output_path_cloned)?.len();
            Ok(size)
        })
        .await
        .map_err(|e| Error::Snapshot(format!("compression task panicked: {}", e)))?
        .map_err(|e| Error::Snapshot(format!("tar+zstd compression failed: {}", e)))?;

        Ok(compressed_size)
    }
}

impl Default for SnapshotCompressor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::metadata::CheckpointMetadata;
    use std::path::Path;
    use tempfile::TempDir;
    use tokio::fs;

    #[test]
    fn test_compressor_creation() {
        let compressor = SnapshotCompressor::new();
        assert_eq!(compressor.zstd_level, 10);
    }

    #[test]
    fn test_compressor_custom_level() {
        let compressor = SnapshotCompressor::new().with_level(15);
        assert_eq!(compressor.zstd_level, 15);
    }

    #[tokio::test]
    async fn test_compress_writes_output() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path().join("ckpt");
        fs::create_dir_all(&base).await.unwrap();

        let meta = CheckpointMetadata::new(10, 1, 0, base.clone());
        let ckpt = base.join(meta.dir_name());
        fs::create_dir_all(&ckpt).await.unwrap();
        fs::write(ckpt.join("a.txt"), b"hello checkpoint")
            .await
            .unwrap();

        let out = tmp.path().join("out").join("snapshot.zst");
        let compressor = SnapshotCompressor::new();
        let size = compressor.compress(&meta, out.clone()).await.unwrap();

        assert!(size > 0);
        assert!(Path::new(&out).exists());
    }
}
