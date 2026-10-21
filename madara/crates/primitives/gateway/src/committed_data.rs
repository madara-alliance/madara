//! Madara-specific, bounded dataset transfer over the feeder gateway.
use serde::{Deserialize, Serialize};
use starknet_types_core::felt::Felt;

pub const ROOTS_PER_PAGE: usize = 64;
pub const VALUES_PER_PAGE: usize = 4096;

/// Operational namespace for a patch. IDs are opaque felts, not access-control identities.
/// `None` builds from empty for initialization or an identical registered-root retry.
/// Otherwise omitted keys survive from `base_root`; namespaces cannot be reset implicitly.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommittedDataUpdate {
    pub application_id: Felt,
    pub dataset_id: Felt,
    #[serde(deserialize_with = "Option::<Felt>::deserialize")]
    pub base_root: Option<Felt>,
}

/// A retained version, ordered lexically for bounded replication. This does not mark a root
/// as published on-chain or imply that it is the current version of the application's contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommittedDataVersion {
    pub application_id: Felt,
    pub dataset_id: Felt,
    pub root: Felt,
}

/// A page is transport data, not authenticated until the receiver verifies the complete root.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommittedDataPage {
    pub root: Felt,
    pub start: u32,
    pub count: u32,
    pub entries: Vec<(Felt, Felt)>,
}
