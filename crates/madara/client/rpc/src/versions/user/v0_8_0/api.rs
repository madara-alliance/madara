use alloy::primitives::TxHash;
use jsonrpsee::core::RpcResult;
use m_proc_macros::versioned_rpc;
use mp_block::BlockId;
use serde::{Deserialize, Serialize};
use starknet_types_core::felt::Felt;
use starknet_types_rpc::TxnStatus;

pub(crate) type NewHead = starknet_types_rpc::BlockHeader<Felt>;
pub(crate) type EmittedEvent = starknet_types_rpc::EmittedEvent<Felt>;

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageStatus {
    pub transaction_hash: Felt,
    pub finality_status: TxnStatus,
    pub failure_reason: Option<String>,
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

    /// For the given L1 transaction hash, return the associated L1 handler transaction hashes and
    /// statuses for all L1 -> L2 messsages sent by the L1 transaction, ordered by the L1
    /// transaction sending order.
    #[method(name = "getMessagesStatus")]
    fn get_messages_status(&self, transaction_hash: TxHash) -> RpcResult<Vec<MessageStatus>>;
}
