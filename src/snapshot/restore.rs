//! Snapshot decompression and recovery for Phase 2
//!
//! This module implements decompression and restoration of checkpoints.
//! Target: <2s for 300MB (decompression + restore).

use super::metadata::CheckpointMetadata;
use crate::error::{Error, Result};
use crc_fast::{CrcAlgorithm, Digest};
use std::io;
use std::path::PathBuf;
use tokio::fs;

/// Restorer for installing compressed snapshots
pub struct SnapshotRestorer;

impl SnapshotRestorer {
    /// Decompress and restore a snapshot
    ///
    /// # Arguments
    /// * `compressed_data` - Compressed snapshot bytes (tar.zst)
    /// * `target_dir` - Directory to restore to
    /// * `expected_meta` - Expected metadata for verification
    ///
    /// # Performance targets
    /// - Decompression + restore: <2s for 300MB
    /// - Atomic operation: all-or-nothing consistency
    pub async fn restore(
        compressed_data: &[u8],
        target_dir: PathBuf,
        expected_meta: &CheckpointMetadata,
    ) -> Result<CheckpointMetadata> {
        if expected_meta.crc32 != 0 && !Self::verify_crc32(compressed_data, expected_meta.crc32) {
            return Err(Error::Snapshot(
                "CRC32 mismatch while restoring snapshot".to_string(),
            ));
        }

        fs::create_dir_all(&target_dir).await?;

        let compressed_owned = compressed_data.to_vec();
        let target_dir_cloned = target_dir.clone();

        tokio::task::spawn_blocking(move || -> io::Result<()> {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| io::Error::other(format!("failed to build runtime: {}", e)))?;

            let tar_tmp = target_dir_cloned.join("snapshot.tar.tmp");
            {
                let mut src = std::io::Cursor::new(compressed_owned);
                let mut dst = std::fs::File::create(&tar_tmp)?;
                zstd::stream::copy_decode(&mut src, &mut dst)?;
            }

            rt.block_on(async {
                let tar_file = tokio::fs::File::open(&tar_tmp).await?;
                let mut archive = tokio_tar::Archive::new(tar_file);
                archive.unpack(&target_dir_cloned).await?;
                io::Result::Ok(())
            })?;

            let _ = std::fs::remove_file(&tar_tmp);
            Ok(())
        })
        .await
        .map_err(|e| Error::Snapshot(format!("restore task panicked: {}", e)))?
        .map_err(|e| Error::Snapshot(format!("tar+zstd restore failed: {}", e)))?;

        let mut restored = expected_meta.clone();
        restored.checkpoint_dir = target_dir;
        Ok(restored)
    }

    /// Verify CRC32 checksum of compressed data
    pub fn verify_crc32(data: &[u8], expected_crc32: u32) -> bool {
        let mut digest = Digest::new(CrcAlgorithm::Crc32IsoHdlc);
        digest.update(data);
        (digest.finalize() as u32) == expected_crc32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::compression::SnapshotCompressor;
    use tempfile::TempDir;

    #[test]
    fn test_crc32_verification() {
        let data = b"abc";
        let mut digest = Digest::new(CrcAlgorithm::Crc32IsoHdlc);
        digest.update(data);
        let crc = digest.finalize() as u32;
        assert!(SnapshotRestorer::verify_crc32(data, crc));
        assert!(!SnapshotRestorer::verify_crc32(data, crc.wrapping_add(1)));
    }

    #[tokio::test]
    async fn test_restore_roundtrip_bundle() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path().join("ckpt");
        fs::create_dir_all(&base).await.unwrap();

        let meta = CheckpointMetadata::new(10, 1, 0, base.clone());
        let ckpt = base.join(meta.dir_name());
        fs::create_dir_all(&ckpt).await.unwrap();
        fs::write(ckpt.join("k.txt"), b"v").await.unwrap();
        fs::create_dir_all(ckpt.join("a").join("b")).await.unwrap();
        fs::write(ckpt.join("a").join("b").join("nested.txt"), b"nested")
            .await
            .unwrap();

        let out = tmp.path().join("s.zst");
        let size = SnapshotCompressor::new()
            .compress(&meta, out.clone())
            .await
            .unwrap();
        assert!(size > 0);

        let compressed = fs::read(out).await.unwrap();
        let restored_dir = tmp.path().join("restored");
        let restored_meta = SnapshotRestorer::restore(&compressed, restored_dir.clone(), &meta)
            .await
            .unwrap();

        let restored_file = restored_dir.join("checkpoint").join("k.txt");
        let restored_data = fs::read(restored_file).await.unwrap();
        assert_eq!(restored_data, b"v");

        let nested_file = restored_dir
            .join("checkpoint")
            .join("a")
            .join("b")
            .join("nested.txt");
        let nested_data = fs::read(nested_file).await.unwrap();
        assert_eq!(nested_data, b"nested");

        assert_eq!(restored_meta.applied_index, meta.applied_index);
    }
}
