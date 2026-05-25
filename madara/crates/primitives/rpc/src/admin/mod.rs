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
pub struct MempoolNonceFilter {
    pub nonce_after: Option<Felt>,
    pub nonce_before: Option<Felt>,
}

const DEFAULT_GET_MEMPOOL_TXN_HASHES_LIMIT: usize = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GetMempoolTxnHashesParams {
    pub include_ttl: bool,
    pub limit: usize,
    #[serde(flatten)]
    pub nonce_filter: MempoolNonceFilter,
}

impl Default for GetMempoolTxnHashesParams {
    fn default() -> Self {
        Self { include_ttl: false, limit: DEFAULT_GET_MEMPOOL_TXN_HASHES_LIMIT, nonce_filter: Default::default() }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GetMempoolTxnsParams {
    #[serde(flatten)]
    pub nonce_filter: MempoolNonceFilter,
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
    #[serde(flatten)]
    pub nonce_filter: MempoolNonceFilter,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlushMempoolTxnsResult {
    pub removed_transaction_hashes: Vec<Felt>,
}
