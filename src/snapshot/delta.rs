//! Delta snapshot serialization and replay helpers.

use crate::error::{Error, Result};
use crate::types::DeltaEntry;
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Codec for delta snapshot payloads.
pub struct DeltaSnapshotCodec;

impl DeltaSnapshotCodec {
    /// Serialize and compress delta entries with postcard + zstd.
    pub fn encode_entries(entries: &[DeltaEntry]) -> Result<Vec<u8>> {
        let encoded = postcard::to_stdvec(entries)?;
        let mut encoder = zstd::Encoder::new(Vec::new(), 3)
            .map_err(|e| Error::Snapshot(format!("create delta encoder failed: {}", e)))?;
        encoder
            .write_all(&encoded)
            .map_err(|e| Error::Snapshot(format!("write delta entries failed: {}", e)))?;

        encoder
            .finish()
            .map_err(|e| Error::Snapshot(format!("finish delta compression failed: {}", e)))
    }

    /// Decompress and deserialize delta entries.
    pub fn decode_entries(data: &[u8]) -> Result<Vec<DeltaEntry>> {
        let mut decoder =
            zstd::Decoder::new(Cursor::new(data)).map_err(|e| Error::Snapshot(e.to_string()))?;
        let mut decompressed = Vec::new();
        decoder
            .read_to_end(&mut decompressed)
            .map_err(|e| Error::Snapshot(format!("read delta payload failed: {}", e)))?;

        postcard::from_bytes(&decompressed)
            .map_err(|e| Error::Snapshot(format!("decode delta entries failed: {}", e)))
    }

    /// Persist compressed delta data to a temp file and return path + size.
    pub async fn persist_to_temp_file(
        base_dir: &Path,
        data: &[u8],
        suffix: &str,
    ) -> Result<PathBuf> {
        tokio::fs::create_dir_all(base_dir).await?;

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let path = base_dir.join(format!("delta-{}-{}.bin.zst", timestamp, suffix));
        tokio::fs::write(&path, data).await?;
        Ok(path)
    }

    /// Read compressed delta bytes from disk.
    pub async fn load_from_file(path: &Path) -> Result<Vec<u8>> {
        let bytes = tokio::fs::read(path).await?;
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_delta_codec_roundtrip() {
        let entries = vec![
            DeltaEntry::new(1, 10, postcard::to_stdvec(&"hello").unwrap()),
            DeltaEntry::new(1, 11, postcard::to_stdvec(&"world").unwrap()),
        ];

        let compressed = DeltaSnapshotCodec::encode_entries(&entries).unwrap();
        assert!(!compressed.is_empty());

        let decoded = DeltaSnapshotCodec::decode_entries(&compressed).unwrap();
        assert_eq!(entries, decoded);
    }
}
