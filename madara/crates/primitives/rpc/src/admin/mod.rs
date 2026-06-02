use serde::{Deserialize, Serialize};
use starknet_types_core::felt::Felt;

use crate::{
    v0_10_2::TxnWithHashAndProofFacts,
    v0_7_1::{Address, DeprecatedContractClass, Signature},
};

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
#[serde(default)]
pub struct MempoolNonceFilter {
    pub nonce_after: Option<Felt>,
    pub nonce_before: Option<Felt>,
}

const DEFAULT_GET_MEMPOOL_TXN_LIMIT: usize = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GetMempoolTxnsParams {
    /// Include the remaining TTL for each transaction when the node has a mempool TTL configured.
    pub include_ttl: bool,
    /// Number of matching transactions to skip before returning results.
    pub offset: usize,
    /// Maximum number of transactions to return.
    pub limit: usize,
    #[serde(flatten)]
    pub nonce_filter: MempoolNonceFilter,
}

impl Default for GetMempoolTxnsParams {
    fn default() -> Self {
        Self { include_ttl: false, offset: 0, limit: DEFAULT_GET_MEMPOOL_TXN_LIMIT, nonce_filter: Default::default() }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MempoolTxnHashInfo {
    pub transaction_hash: Felt,
    pub remaining_ttl_ms: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MempoolTxnInfo {
    #[serde(flatten)]
    pub transaction: TxnWithHashAndProofFacts,
    /// Remaining mempool TTL in milliseconds.
    pub remaining_ttl_ms: Option<u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub struct FlushMempoolTxnsParams {
    pub all: bool,
    /// Optional sender/account contract address to flush.
    pub contract_address: Option<Felt>,
    pub transaction_hashes: Option<Vec<Felt>>,
    /// Optional nonce range used to narrow the explicit base selector above.
    /// This filter is invalid unless `all`, `contract_address`, or `transaction_hashes` is also provided.
    /// Warning: removing a non-tail nonce can leave higher nonces from the same account pending in
    /// the mempool until they are flushed separately or displaced by account state changes.
    #[serde(flatten)]
    pub nonce_filter: MempoolNonceFilter,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlushMempoolTxnsResult {
    pub removed_transaction_hashes: Vec<Felt>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn flush_mempool_txns_params_reject_unknown_fields() {
        let error = serde_json::from_value::<FlushMempoolTxnsParams>(json!({
            "all": true,
            "contract_address_field": "to",
        }))
        .expect_err("unexpected fields should be rejected");

        assert!(error.to_string().contains("unknown field `contract_address_field`"));
    }
}
