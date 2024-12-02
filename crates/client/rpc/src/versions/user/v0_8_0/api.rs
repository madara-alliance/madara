use jsonrpsee::core::RpcResult;
use m_proc_macros::versioned_rpc;
use mp_block::BlockId;
use serde::{Deserialize, Serialize};
use serde_with::serde_as;
use starknet_core::serde::unsigned_field_element::UfeHex;
use starknet_types_core::felt::Felt;

pub(crate) type NewHead = starknet_types_rpc::BlockHeader<Felt>;

#[serde_as]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContractStorageKeysItem {
    #[serde_as(as = "UfeHex")]
    pub contract_address: Felt,
    #[serde_as(as = "Vec<UfeHex>")]
    pub storage_keys: Vec<Felt>,
}

#[serde_as]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MerkleNode {
    Binary {
        #[serde_as(as = "UfeHex")]
        left: Felt,
        #[serde_as(as = "UfeHex")]
        right: Felt,
    },
    Edge {
        #[serde_as(as = "UfeHex")]
        child: Felt,
        #[serde_as(as = "UfeHex")]
        path: Felt,
        length: usize,
    },
}

#[serde_as]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeHashToNodeMappingItem {
    #[serde_as(as = "UfeHex")]
    pub node_hash: Felt,
    pub node: MerkleNode,
}

#[serde_as]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContractLeavesDataItem {
    #[serde_as(as = "UfeHex")]
    pub nonce: Felt,
    #[serde_as(as = "UfeHex")]
    pub class_hash: Felt,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContractsProof {
    pub nodes: Vec<NodeHashToNodeMappingItem>,
    pub contract_leaves_data: Vec<ContractLeavesDataItem>,
}

#[serde_as]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GlobalRoots {
    #[serde_as(as = "UfeHex")]
    pub contracts_tree_root: Felt,
    #[serde_as(as = "UfeHex")]
    pub classes_tree_root: Felt,
    #[serde_as(as = "UfeHex")]
    pub block_hash: Felt,
}

#[serde_as]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetStorageProofResult {
    pub classes_proof: Vec<NodeHashToNodeMappingItem>,
    pub contracts_proof: ContractsProof,
    pub contracts_storage_proofs: Vec<Vec<NodeHashToNodeMappingItem>>,
    pub global_roots: GlobalRoots,
}

#[versioned_rpc("V0_8_0", "starknet")]
pub trait StarknetWsRpcApi {
    #[subscription(name = "subscribeNewHeads", unsubscribe = "unsubscribe", item = NewHead, param_kind = map)]
    async fn subscribe_new_heads(&self, block_id: BlockId) -> jsonrpsee::core::SubscriptionResult;
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
