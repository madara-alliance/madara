pub const JOBS_COLLECTION: &str = "jobs";

/// Collection name for SNOS batches
///
/// SNOS batches represent individual batch units that contain block ranges
/// for SNOS (Starknet OS) job processing. Each SNOS batch belongs to an
/// aggregator batch and has a unique sequential ID.
pub const SNOS_BATCHES_COLLECTION: &str = "snos_batches";

/// Collection name for Aggregator batches
///
/// Aggregator batches represent higher-level batch units that contain
/// multiple SNOS batches. They manage the aggregation process and
/// maintain state for proof generation and settlement.
pub const AGGREGATOR_BATCHES_COLLECTION: &str = "aggregator_batches";

/// Legacy collection name for batches (deprecated)
///
/// This was previously used for storing both SNOS and Aggregator batches
/// in a single collection. Now deprecated in favor of separate collections.
/// Kept for backward compatibility during migration period.
#[deprecated(note = "Use SNOS_BATCHES_COLLECTION or AGGREGATOR_BATCHES_COLLECTION instead")]
pub const BATCHES_COLLECTION: &str = "batches";
