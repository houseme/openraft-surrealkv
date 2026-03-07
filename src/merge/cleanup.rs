use crate::error::{Error, Result};
use crate::state::SnapshotMetaState;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::fs;

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Cleanup tuning knobs for merge artifacts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeCleanupConfig {
    /// Keep at most this many latest checkpoint directories.
    pub retain_last_checkpoints: usize,
    /// Remove delta files older than this threshold.
    pub max_delta_age_secs: u64,
    /// Remove temp merge dirs older than this threshold.
    pub max_temp_age_secs: u64,
    /// Base directory that stores checkpoints.
    pub checkpoints_dir: PathBuf,
    /// Base directory that stores temporary merge artifacts.
    pub temp_merge_dir: PathBuf,
}

impl Default for MergeCleanupConfig {
    fn default() -> Self {
        Self {
            retain_last_checkpoints: 3,
            max_delta_age_secs: 24 * 60 * 60,
            max_temp_age_secs: 60 * 60,
            checkpoints_dir: PathBuf::from("target/checkpoints"),
            temp_merge_dir: PathBuf::from("target/tmp/merge"),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CleanupReport {
    pub removed_delta_files: usize,
    pub removed_temp_dirs: usize,
    pub removed_checkpoint_dirs: usize,
}

/// Merge cleanup implementation.
#[derive(Debug, Clone, Default)]
pub struct MergeCleanup {
    pub config: MergeCleanupConfig,
}

impl MergeCleanup {
    pub fn new(config: MergeCleanupConfig) -> Self {
        Self { config }
    }

    pub async fn run(
        &self,
        snapshot_state: &SnapshotMetaState,
        now: Option<u64>,
    ) -> Result<CleanupReport> {
        let now = now.unwrap_or_else(now_secs);
        let mut report = CleanupReport::default();

        report.removed_delta_files = self.cleanup_delta_files(snapshot_state, now).await?;
        report.removed_temp_dirs = self.cleanup_temp_dirs(now).await?;
        report.removed_checkpoint_dirs = self.cleanup_old_checkpoints().await?;

        Ok(report)
    }

    async fn cleanup_delta_files(
        &self,
        snapshot_state: &SnapshotMetaState,
        now: u64,
    ) -> Result<usize> {
        let mut removed = 0usize;

        for delta in &snapshot_state.delta_chain {
            let Some(path) = &delta.file_path else {
                continue;
            };

            let age = now.saturating_sub(delta.created_at);
            if age < self.config.max_delta_age_secs {
                continue;
            }

            if fs::remove_file(path).await.is_ok() {
                removed = removed.saturating_add(1);
            }
        }

        Ok(removed)
    }

    async fn cleanup_temp_dirs(&self, now: u64) -> Result<usize> {
        if !self.config.temp_merge_dir.exists() {
            return Ok(0);
        }

        let mut removed = 0usize;
        let mut rd = fs::read_dir(&self.config.temp_merge_dir)
            .await
            .map_err(Error::Io)?;

        while let Some(entry) = rd.next_entry().await.map_err(Error::Io)? {
            let meta = entry.metadata().await.map_err(Error::Io)?;
            if !meta.is_dir() {
                continue;
            }

            let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            let modified_secs = modified
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);

            if now.saturating_sub(modified_secs) >= self.config.max_temp_age_secs {
                fs::remove_dir_all(entry.path())
                    .await
                    .map_err(Error::Io)?;
                removed = removed.saturating_add(1);
            }
        }

        Ok(removed)
    }

    async fn cleanup_old_checkpoints(&self) -> Result<usize> {
        if !self.config.checkpoints_dir.exists() {
            return Ok(0);
        }

        let mut dirs = Vec::new();
        let mut rd = fs::read_dir(&self.config.checkpoints_dir)
            .await
            .map_err(Error::Io)?;

        while let Some(entry) = rd.next_entry().await.map_err(Error::Io)? {
            let meta = entry.metadata().await.map_err(Error::Io)?;
            if !meta.is_dir() {
                continue;
            }

            let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            dirs.push((entry.path(), modified));
        }

        if dirs.len() <= self.config.retain_last_checkpoints {
            return Ok(0);
        }

        // Remove oldest first, keep newest N.
        dirs.sort_by_key(|(_, modified)| *modified);
        let remove_count = dirs.len() - self.config.retain_last_checkpoints;

        for (path, _) in dirs.iter().take(remove_count) {
            remove_dir_if_exists(path).await?;
        }

        Ok(remove_count)
    }
}

async fn remove_dir_if_exists(path: &Path) -> Result<()> {
    if path.exists() {
        fs::remove_dir_all(path).await.map_err(Error::Io)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::DeltaInfo;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_cleanup_expired_delta_file() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("delta.bin");
        fs::write(&p, b"x").await.unwrap();

        let mut state = SnapshotMetaState::new();
        state
            .delta_chain
            .push(DeltaInfo::new(1, 2, 1, 10).with_file_path(p.display().to_string()));

        let cleanup = MergeCleanup::new(MergeCleanupConfig {
            max_delta_age_secs: 5,
            ..MergeCleanupConfig::default()
        });

        let removed = cleanup.cleanup_delta_files(&state, 20).await.unwrap();
        assert_eq!(removed, 1);
        assert!(!p.exists());
    }
}
