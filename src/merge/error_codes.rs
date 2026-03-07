/// Stable merge error codes for logs, metrics labels and alerting.
/// These should be used for all errors returned by the merge executor and cleanup, and recorded in metrics with the `merge_strict_error_total` counter metric.

pub const MERGE_ERR_BASELINE_MISSING: &str = "MERGE_BASELINE_MISSING";
pub const MERGE_ERR_BASELINE_PATH_REQUIRED: &str = "MERGE_BASELINE_PATH_REQUIRED";
pub const MERGE_ERR_BASELINE_PATH_MISSING: &str = "MERGE_BASELINE_PATH_MISSING";
pub const MERGE_ERR_INJECTED_FAILURE: &str = "MERGE_INJECTED_FAILURE";
pub const MERGE_ERR_UNKNOWN: &str = "MERGE_UNKNOWN";
