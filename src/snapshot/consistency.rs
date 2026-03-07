//! Consistency helpers for snapshot/checkpoint metadata.
//!
//! Priority for resolving checkpoint sequence number:
//! 1. SurrealKV native checkpoint metadata
//! 2. SurrealKV fixed internal key (`raft_consistency:sequence_number`)
//! 3. Checkpoint manifest parsing (CHECKPOINT_METADATA)
//!
//! Write strategy: both SurrealKV key and checkpoint manifest (dual persistence).

use crate::error::{Error, Result};
use std::path::Path;
use std::sync::Arc;
use surrealkv::Tree;

const CHECKPOINT_METADATA_FILE: &str = "CHECKPOINT_METADATA";
const CONSISTENCY_SEQ_KEY: &[u8] = b"raft_consistency:sequence_number";

/// Interface for reading a consistency sequence number from storage.
pub trait SequenceNumberReader: Send + Sync {
    /// Read sequence number from storage for checkpoint consistency.
    fn read_sequence_number(&self, tree: &Arc<Tree>) -> Result<u64>;
}

/// Interface for writing a consistency sequence number to storage.
pub trait SequenceNumberWriter: Send + Sync {
    /// Write sequence number to storage for checkpoint consistency.
    fn write_sequence_number(&self, tree: &Arc<Tree>, sequence_number: u64) -> Result<()>;
}

/// Reader that loads the sequence number from a persisted consistency key.
pub struct PersistedKeySequenceNumberReader;

impl SequenceNumberReader for PersistedKeySequenceNumberReader {
    fn read_sequence_number(&self, tree: &Arc<Tree>) -> Result<u64> {
        let mut value: Option<Vec<u8>> = None;
        tree.view(|txn| {
            value = txn.get(CONSISTENCY_SEQ_KEY)?;
            Ok(())
        })
        .map_err(|e| Error::Snapshot(format!("read consistency key failed: {}", e)))?;

        let raw = value
            .ok_or_else(|| Error::Snapshot("persisted consistency key is missing".to_string()))?;

        if raw.len() == 8 {
            let mut bytes = [0u8; 8];
            bytes.copy_from_slice(&raw);
            return Ok(u64::from_be_bytes(bytes));
        }

        if let Ok(s) = std::str::from_utf8(&raw)
            && let Ok(v) = s.parse::<u64>()
        {
            return Ok(v);
        }

        Err(Error::Snapshot(
            "persisted consistency key has unsupported format".to_string(),
        ))
    }
}

/// Writer that persists the sequence number to SurrealKV fixed internal key.
pub struct PersistedKeySequenceNumberWriter;

impl SequenceNumberWriter for PersistedKeySequenceNumberWriter {
    fn write_sequence_number(&self, tree: &Arc<Tree>, sequence_number: u64) -> Result<()> {
        let value_bytes = sequence_number.to_be_bytes();
        let tree = tree.clone();

        let handle = std::thread::spawn(move || -> Result<()> {
            let mut txn = tree
                .begin()
                .map_err(|e| Error::Snapshot(format!("begin transaction failed: {}", e)))?;

            txn.set(CONSISTENCY_SEQ_KEY, value_bytes.as_slice())
                .map_err(|e| Error::Snapshot(format!("set consistency key failed: {}", e)))?;

            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| {
                    Error::Snapshot(format!("build consistency write runtime failed: {}", e))
                })?;

            rt.block_on(txn.commit())
                .map_err(|e| Error::Snapshot(format!("commit consistency key failed: {}", e)))?;

            Ok(())
        });

        handle
            .join()
            .map_err(|_| Error::Snapshot("consistency writer thread panicked".to_string()))?
    }
}

/// Parse sequence number from SurrealKV checkpoint manifest file.
pub fn parse_sequence_from_checkpoint_manifest(checkpoint_dir: &Path) -> Result<Option<u64>> {
    let p = checkpoint_dir.join(CHECKPOINT_METADATA_FILE);
    if !p.exists() {
        return Ok(None);
    }

    let data = std::fs::read(&p)
        .map_err(|e| Error::Snapshot(format!("read checkpoint manifest failed: {}", e)))?;

    // Format: version(u32) + timestamp(u64) + sequence_number(u64) + ... (big-endian)
    if data.len() < 20 {
        return Ok(None);
    }

    let mut seq_bytes = [0u8; 8];
    seq_bytes.copy_from_slice(&data[12..20]);
    Ok(Some(u64::from_be_bytes(seq_bytes)))
}

/// Write sequence number to both SurrealKV key and checkpoint manifest (dual persistence).
pub fn write_sequence_number_dual(
    tree: &Arc<Tree>,
    checkpoint_dir: &Path,
    sequence_number: u64,
    writer: &dyn SequenceNumberWriter,
) -> Result<()> {
    // Write to SurrealKV fixed key (priority source).
    writer.write_sequence_number(tree, sequence_number)?;

    // Also write to checkpoint manifest for redundancy.
    let manifest_path = checkpoint_dir.join(CHECKPOINT_METADATA_FILE);
    if let Some(parent) = manifest_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| Error::Snapshot(format!("create checkpoint dir failed: {}", e)))?;
    }

    // Build minimal manifest: version(u32) + timestamp(u64) + sequence_number(u64)
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let mut manifest_bytes = Vec::with_capacity(20);
    manifest_bytes.extend_from_slice(&1u32.to_be_bytes()); // version
    manifest_bytes.extend_from_slice(&timestamp.to_be_bytes());
    manifest_bytes.extend_from_slice(&sequence_number.to_be_bytes());

    std::fs::write(&manifest_path, &manifest_bytes)
        .map_err(|e| Error::Snapshot(format!("write checkpoint manifest failed: {}", e)))?;

    Ok(())
}

/// Resolve sequence number with priority:
/// 1) value returned by SurrealKV checkpoint API
/// 2) SurrealKV fixed internal key
/// 3) checkpoint manifest parsing
pub fn resolve_sequence_number(
    from_checkpoint: Option<u64>,
    checkpoint_dir: &Path,
    tree: &Arc<Tree>,
    reader: &dyn SequenceNumberReader,
) -> Result<u64> {
    if let Some(seq) = from_checkpoint {
        return Ok(seq);
    }

    // Try SurrealKV fixed key first (priority over manifest).
    if let Ok(seq) = reader.read_sequence_number(tree) {
        return Ok(seq);
    }

    // Fallback to checkpoint manifest parsing.
    if let Some(seq) = parse_sequence_from_checkpoint_manifest(checkpoint_dir)? {
        return Ok(seq);
    }

    Err(Error::Snapshot(
        "sequence number unavailable from all sources".to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use surrealkv::TreeBuilder;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_dual_persistence_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let tree = Arc::new(
            TreeBuilder::new()
                .with_path(tmp.path().join("db"))
                .build()
                .unwrap(),
        );

        let writer = PersistedKeySequenceNumberWriter;
        let reader = PersistedKeySequenceNumberReader;

        let checkpoint_dir = tmp.path().join("checkpoints");
        let seq = 12345u64;

        write_sequence_number_dual(&tree, &checkpoint_dir, seq, &writer).unwrap();

        // Verify SurrealKV key.
        let read_seq = reader.read_sequence_number(&tree).unwrap();
        assert_eq!(read_seq, seq);

        // Verify checkpoint manifest.
        let manifest_seq = parse_sequence_from_checkpoint_manifest(&checkpoint_dir)
            .unwrap()
            .unwrap();
        assert_eq!(manifest_seq, seq);
    }

    #[tokio::test]
    async fn test_resolve_sequence_fallback_to_manifest_when_key_missing() {
        let tmp = TempDir::new().unwrap();
        let tree = Arc::new(
            TreeBuilder::new()
                .with_path(tmp.path().join("db-fallback-manifest"))
                .build()
                .unwrap(),
        );

        let checkpoint_dir = tmp.path().join("checkpoint-dir");
        std::fs::create_dir_all(&checkpoint_dir).unwrap();

        let expected = 424242u64;
        // Write manifest only (no persisted key) to simulate a missing-key scenario.
        let mut manifest = Vec::with_capacity(20);
        manifest.extend_from_slice(&1u32.to_be_bytes());
        manifest.extend_from_slice(&1u64.to_be_bytes());
        manifest.extend_from_slice(&expected.to_be_bytes());
        std::fs::write(checkpoint_dir.join("CHECKPOINT_METADATA"), manifest).unwrap();

        let reader = PersistedKeySequenceNumberReader;
        let seq = resolve_sequence_number(None, &checkpoint_dir, &tree, &reader).unwrap();
        assert_eq!(seq, expected);
    }

    #[tokio::test]
    async fn test_resolve_sequence_uses_metadata_even_when_key_missing() {
        let tmp = TempDir::new().unwrap();
        let tree = Arc::new(
            TreeBuilder::new()
                .with_path(tmp.path().join("db-fallback-metadata"))
                .build()
                .unwrap(),
        );

        let checkpoint_dir = tmp.path().join("checkpoint-dir");
        std::fs::create_dir_all(&checkpoint_dir).unwrap();

        let reader = PersistedKeySequenceNumberReader;
        let seq = resolve_sequence_number(Some(777), &checkpoint_dir, &tree, &reader).unwrap();
        assert_eq!(seq, 777);
    }
}
