use crate::versions::user::v0_8_1::StarknetReadRpcApiV0_8_1Server;
use crate::Starknet;
use jsonrpsee::core::{async_trait, RpcResult};
use mp_block::BlockId;
use mp_chain_config::RpcVersion;
use mp_rpc::v0_8_1::{ContractStorageKeysItem, GetStorageProofResult};
use starknet_types_core::felt::Felt;

pub mod get_compiled_casm;
pub mod get_storage_proof;

#[async_trait]
impl StarknetReadRpcApiV0_8_1Server for Starknet {
    fn spec_version(&self) -> RpcResult<String> {
        Ok(RpcVersion::RPC_VERSION_0_8_1.to_string())
    }

    fn get_compiled_casm(&self, class_hash: Felt) -> RpcResult<serde_json::Value> {
        Ok(get_compiled_casm::get_compiled_casm(self, class_hash)?)
    }

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
