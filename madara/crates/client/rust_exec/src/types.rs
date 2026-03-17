//! Core types for Rust native execution.
//!
//! These types mirror Blockifier's output types to enable comparison.

use indexmap::IndexMap;
use serde::Serialize;
use starknet_types_core::felt::Felt;

/// Storage key (same as Blockifier's StorageKey)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub struct StorageKey(pub Felt);

impl From<Felt> for StorageKey {
    fn from(felt: Felt) -> Self {
        Self(felt)
    }
}

/// Contract address
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub struct ContractAddress(pub Felt);

impl From<Felt> for ContractAddress {
    fn from(felt: Felt) -> Self {
        Self(felt)
    }
}

/// Nonce
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Nonce(pub Felt);

impl Nonce {
    pub fn increment(&self) -> Self {
        Self(self.0 + Felt::ONE)
    }
}

/// An emitted event
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Event {
    /// Order in which this event was emitted
    pub order: usize,
    /// Event keys (first key is typically the event selector)
    pub keys: Vec<Felt>,
    /// Event data
    pub data: Vec<Felt>,
}

/// L2 to L1 message
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct L2ToL1Message {
    /// L1 recipient address
    pub to_address: Felt,
    /// Message payload
    pub payload: Vec<Felt>,
}

/// Execution result for a single function call
#[derive(Debug, Clone, Default, Serialize)]
pub struct CallExecutionResult {
    /// Return data from the function
    pub retdata: Vec<Felt>,
    /// Events emitted during execution
    pub events: Vec<Event>,
    /// Messages sent to L1
    pub l2_to_l1_messages: Vec<L2ToL1Message>,
    /// Whether execution failed
    pub failed: bool,
    /// Estimated gas consumed
    pub gas_consumed: u64,
}

/// State diff produced by execution
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct StateDiff {
    /// Storage updates: contract -> (key -> new_value)
    #[serde(with = "state_diff_storage_serde")]
    pub storage_updates: IndexMap<ContractAddress, IndexMap<StorageKey, Felt>>,
    /// Nonce updates: contract -> new_nonce
    pub address_to_nonce: IndexMap<ContractAddress, Nonce>,
    /// Class hash updates (for deploys/upgrades)
    pub address_to_class_hash: IndexMap<ContractAddress, Felt>,
    /// Compiled class hash mapping (for declares)
    pub class_hash_to_compiled_class_hash: IndexMap<Felt, Felt>,
}

mod state_diff_storage_serde {
    use super::{ContractAddress, Felt, StorageKey};
    use indexmap::IndexMap;
    use serde::{Serialize, Serializer};

    #[derive(Serialize)]
    struct StorageEntrySerde {
        contract_address: ContractAddress,
        key: StorageKey,
        value: Felt,
    }

    pub fn serialize<S>(
        storage: &IndexMap<ContractAddress, IndexMap<StorageKey, Felt>>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let entries = storage
            .iter()
            .flat_map(|(contract_address, storage_map)| {
                storage_map.iter().map(|(key, value)| StorageEntrySerde {
                    contract_address: *contract_address,
                    key: *key,
                    value: *value,
                })
            })
            .collect::<Vec<_>>();

        entries.serialize(serializer)
    }

    // Deserialize intentionally omitted: we only need JSON output for logging.
}

impl StateDiff {
    /// Merge another StateDiff into this one.
    /// Later writes override earlier writes for the same keys.
    pub fn merge(&mut self, other: StateDiff) {
        // Merge storage updates
        for (contract, storage) in other.storage_updates {
            self.storage_updates.entry(contract).or_default().extend(storage);
        }

        // Merge nonce updates (later nonces override)
        self.address_to_nonce.extend(other.address_to_nonce);

        // Merge class hash updates
        self.address_to_class_hash.extend(other.address_to_class_hash);

        // Merge compiled class hash mappings
        self.class_hash_to_compiled_class_hash.extend(other.class_hash_to_compiled_class_hash);
    }
}

/// Complete execution result
#[derive(Debug, Clone, Serialize)]
pub struct ExecutionResult {
    /// Results from the main execution
    pub call_result: CallExecutionResult,
    /// State changes
    pub state_diff: StateDiff,
    /// Revert error message if execution reverted
    pub revert_error: Option<String>,
}

impl ExecutionResult {
    /// Check if execution was successful
    pub fn is_success(&self) -> bool {
        !self.call_result.failed && self.revert_error.is_none()
    }
}
