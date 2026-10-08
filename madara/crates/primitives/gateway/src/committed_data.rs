//! Madara-specific, bounded dataset transfer over the feeder gateway.
use serde::{Deserialize, Serialize};
use starknet_types_core::felt::Felt;

pub const ROOTS_PER_PAGE: usize = 64;
pub const VALUES_PER_PAGE: usize = 4096;
pub const MAX_DATASET_VALUES: usize = 1 << 19;

/// A page is transport data, not authenticated until the receiver verifies the complete root.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommittedDataPage {
    pub root: Felt,
    pub start: u32,
    pub count: u32,
    pub values: Vec<Felt>,
}
