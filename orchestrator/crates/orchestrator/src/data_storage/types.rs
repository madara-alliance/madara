use std::collections::HashMap;

use cairo_vm::Felt252;
use serde::{Deserialize, Serialize};

/// This struct represents the contract changes that will be in `StarknetOsOutput`
/// as a vector.
#[derive(Debug, Deserialize, Serialize)]
pub struct ContractChanges {
    /// The address of the contract.
    pub addr: Felt252,
    /// The new nonce of the contract (for account contracts).
    pub nonce: Felt252,
    /// The new class hash (if changed).
    pub class_hash: Option<Felt252>,
    /// A map from storage key to its new value.
    pub storage_changes: HashMap<Felt252, Felt252>,
}

/// This struct represents the starknet OS outputs in the json we will get after the run.
#[derive(Debug, Deserialize, Serialize)]
pub struct StarknetOsOutput {
    /// The root before.
    pub initial_root: Felt252,
    /// The root after.
    pub final_root: Felt252,
    /// The block number.
    pub block_number: Felt252,
    /// The block hash.
    pub block_hash: Felt252,
    /// The hash of the OS config.
    pub starknet_os_config_hash: Felt252,
    /// Whether KZG data availability was used.
    pub use_kzg_da: Felt252,
    /// Messages from L2 to L1.
    pub messages_to_l1: Vec<Felt252>,
    /// Messages from L1 to L2.
    pub messages_to_l2: Vec<Felt252>,
    /// The list of contracts that were changed.
    pub contracts: Vec<ContractChanges>,
    /// The list of classes that were declared. A map from class hash to compiled class hash.
    pub classes: HashMap<Felt252, Felt252>,
}
