use starknet_core::types::Felt;

#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct UnverifiedCommitments {
    pub transaction_count: Option<u64>,
    pub transaction_commitment: Option<Felt>,
    pub event_count: Option<u64>,
    pub event_commitment: Option<Felt>,
    pub state_diff_length: Option<u64>,
    pub state_diff_commitment: Option<Felt>,
    pub receipt_commitment: Option<Felt>,
    /// Global state root
    pub global_state_root: Option<Felt>,
    /// Expected block hash
    pub block_hash: Option<Felt>,
}

// Pre-validate outputs.

#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct ValidatedCommitments {
    pub transaction_count: u64,
    pub transaction_commitment: Felt,
    pub event_count: u64,
    pub event_commitment: Felt,
    pub state_diff_length: u64,
    pub state_diff_commitment: Felt,
    pub receipt_commitment: Felt,
}
