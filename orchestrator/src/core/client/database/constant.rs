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

/// Collection name for per-block reverse lookups into batch membership.
///
/// Each document is keyed by a concrete block number and stores the SNOS and
/// aggregator batch indices that currently own that block.
pub const BLOCK_BATCH_LOOKUPS_COLLECTION: &str = "block_batch_lookups";

/// Legacy collection name for batches (deprecated)
///
/// This was previously used for storing both SNOS and Aggregator batches
/// in a single collection. Now deprecated in favor of separate collections.
/// Kept for backward compatibility during migration period.
#[deprecated(note = "Use SNOS_BATCHES_COLLECTION or AGGREGATOR_BATCHES_COLLECTION instead")]
pub const BATCHES_COLLECTION: &str = "batches";
