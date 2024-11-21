use super::methods::read::get_storage_proof;
use jsonrpsee::core::{async_trait, RpcResult};
use m_proc_macros::versioned_rpc;
use serde::{Deserialize, Serialize};
use serde_with::serde_as;
use starknet_core::serde::unsigned_field_element::UfeHex;
use starknet_core::types::BlockId;
use starknet_types_core::felt::Felt;

pub(crate) type NewHead = starknet_types_rpc::BlockHeader<Felt>;


#[async_trait]
impl StarknetReadRpcApiV0_8_0Server for crate::Starknet {
    fn get_storage_proof(
        &self,
        block_id: BlockId,
        class_hashes: Option<Vec<Felt>>,
        contract_addresses: Option<Vec<Felt>>,
        contracts_storage_keys: Option<Vec<ContractStorageKeysItem>>,
    ) -> RpcResult<GetStorageProofResult> {
        get_storage_proof::get_storage_proof(self, block_id, class_hashes, contract_addresses, contracts_storage_keys)
    }
}
