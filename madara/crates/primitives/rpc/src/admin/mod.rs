use serde::{Deserialize, Serialize};
use starknet_types_core::felt::Felt;

use crate::v0_7_1::{Address, DeprecatedContractClass, Signature};

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct BroadcastedDeclareTxnV0 {
    /// The class to be declared
    pub contract_class: DeprecatedContractClass,
    /// The maximal fee that can be charged for including the transaction
    pub max_fee: Felt,
    /// The address of the account contract sending the declaration transaction
    pub sender_address: Address,
    pub signature: Signature,
    pub is_query: bool,
}

impl BroadcastedDeclareTxnV0 {
    pub fn is_query(&self) -> bool {
        self.is_query
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReplayBlockBoundary {
    pub block_n: u64,
    pub expected_tx_count: u64,
    pub last_tx_hash: Felt,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReplayBlockBoundaryStatus {
    pub block_n: u64,
    pub expected_tx_count: u64,
    pub dispatched_tx_count: u64,
    pub executed_tx_count: u64,
    pub last_executed_tx_hash: Option<Felt>,
    pub reached_last_tx_hash: bool,
    pub boundary_met: bool,
    pub closed: bool,
    pub mismatch: Option<String>,
}
