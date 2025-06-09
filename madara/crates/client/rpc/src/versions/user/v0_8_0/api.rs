use jsonrpsee::core::RpcResult;
use m_proc_macros::versioned_rpc;
use mp_block::BlockId;
use serde::{Deserialize, Serialize};
use starknet_types_core::felt::Felt;

pub(crate) type NewHead = mp_rpc::BlockHeader;
pub(crate) type EmittedEvent = mp_rpc::EmittedEvent;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContractStorageKeysItem {
    pub contract_address: Felt,
    pub storage_keys: Vec<Felt>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MerkleNode {
    Binary { left: Felt, right: Felt },
    Edge { child: Felt, path: Felt, length: usize },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeHashToNodeMappingItem {
    pub node_hash: Felt,
    pub node: MerkleNode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContractLeavesDataItem {
    pub nonce: Felt,
    pub class_hash: Felt,
    pub storage_root: Felt,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContractsProof {
    pub nodes: Vec<NodeHashToNodeMappingItem>,
    pub contract_leaves_data: Vec<ContractLeavesDataItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GlobalRoots {
    pub contracts_tree_root: Felt,
    pub classes_tree_root: Felt,
    pub block_hash: Felt,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetStorageProofResult {
    pub classes_proof: Vec<NodeHashToNodeMappingItem>,
    pub contracts_proof: ContractsProof,
    pub contracts_storage_proofs: Vec<Vec<NodeHashToNodeMappingItem>>,
    pub global_roots: GlobalRoots,
}

#[versioned_rpc("V0_8_0", "starknet")]
pub trait StarknetWsRpcApi {
    #[subscription(name = "subscribeNewHeads", unsubscribe = "unsubscribeNewHeads", item = NewHead, param_kind = map)]
    async fn subscribe_new_heads(&self, block: BlockId) -> jsonrpsee::core::SubscriptionResult;

    #[subscription(name = "subscribeEvents", unsubscribe = "unsubscribeEvents", item = EmittedEvent, param_kind = map)]
    async fn subscribe_events(
        &self,
        from_address: Option<Felt>,
        keys: Option<Vec<Vec<Felt>>>,
        block: Option<BlockId>,
    ) -> jsonrpsee::core::SubscriptionResult;

    #[subscription(
        name = "subscribeTransactionStatus",
        unsubscribe = "unsubscribeTransactionStatus",
        item = mp_rpc::v0_8_1::TxnStatus,
        param_kind = map
    )]
    async fn subscribe_transaction_status(&self, transaction_hash: Felt) -> jsonrpsee::core::SubscriptionResult;

    #[subscription(
        name = "subscribePendingTransactions",
        unsubscribe = "unsubscribePendingTransactions",
        item = mp_rpc::v0_8_1::TxnStatus,
        param_kind = map
    )]
    async fn subscribe_pending_transactions(
        &self,
        transaction_defailts: bool,
        sender_address: Vec<starknet_types_core::felt::Felt>,
    ) -> jsonrpsee::core::SubscriptionResult;
}

#[versioned_rpc("V0_8_0", "starknet")]
pub trait StarknetReadRpcApi {
    #[method(name = "specVersion")]
    fn spec_version(&self) -> RpcResult<String>;

    #[method(name = "getCompiledCasm")]
    fn get_compiled_casm(&self, class_hash: Felt) -> RpcResult<serde_json::Value>;

    #[method(name = "getStorageProof")]
    fn get_storage_proof(
        &self,
        block_id: BlockId,
        class_hashes: Option<Vec<Felt>>,
        contract_addresses: Option<Vec<Felt>>,
        contracts_storage_keys: Option<Vec<ContractStorageKeysItem>>,
    ) -> RpcResult<GetStorageProofResult>;
}
