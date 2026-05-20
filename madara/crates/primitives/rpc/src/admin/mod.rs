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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MempoolContractAddressField {
    #[default]
    Sender,
    To,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GetMempoolTxnHashesParams {
    pub include_ttl: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MempoolTxnHashInfo {
    pub transaction_hash: Felt,
    pub remaining_ttl_ms: Option<u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FlushMempoolTxnsParams {
    pub all: bool,
    pub contract_address: Option<Felt>,
    pub contract_address_field: Option<MempoolContractAddressField>,
    pub transaction_hashes: Option<Vec<Felt>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlushMempoolTxnsResult {
    pub removed_transaction_hashes: Vec<Felt>,
}
