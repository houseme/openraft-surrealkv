//! Hybrid snapshot merge strategy (Phase 4).
//!
//! This module provides:
//! - 3D merge policy decisions (`policy`)
//! - async background merge orchestration (`executor`)
//! - stale file and directory cleanup (`cleanup`)

pub mod cleanup {
    include!("merge/cleanup.rs");
}

pub mod executor {
    include!("merge/executor.rs");
}

pub mod policy {
    include!("merge/policy.rs");
}

pub mod error_codes {
    include!("merge/error_codes.rs");
}

pub mod checkpoint_backend {
    include!("merge/checkpoint_backend.rs");
}

pub use checkpoint_backend::CheckpointMergeBackend;
pub use cleanup::{CleanupReport, MergeCleanup, MergeCleanupConfig};
pub use error_codes::*;
pub use executor::{MergeBackend, MergeExecution, MergeExecutor, MergeTaskHandle, MergeTaskResult};
pub use policy::{DeltaMergePolicy, MergeDecision, MergeTrigger};
