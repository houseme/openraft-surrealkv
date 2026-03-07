// Stable merge error-code strings used by logs, metrics labels, and alerting.

/// Define the error code for missing baseline checkpoint metadata.
pub const MERGE_ERR_BASELINE_MISSING: &str = "MERGE_BASELINE_MISSING";
/// Define the error code for missing explicit baseline checkpoint path in metadata.
pub const MERGE_ERR_BASELINE_PATH_REQUIRED: &str = "MERGE_BASELINE_PATH_REQUIRED";
/// Define the error code for baseline checkpoint path that is missing on disk.
pub const MERGE_ERR_BASELINE_PATH_MISSING: &str = "MERGE_BASELINE_PATH_MISSING";
/// Define the error code used by injected-failure test paths.
pub const MERGE_ERR_INJECTED_FAILURE: &str = "MERGE_INJECTED_FAILURE";
/// Define the fallback error code for uncategorized merge failures.
pub const MERGE_ERR_UNKNOWN: &str = "MERGE_UNKNOWN";
