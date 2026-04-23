use crate::versions::user::common::convert_storage_keys_for_v0_8_1;
use crate::versions::user::v0_8_1::methods::read::get_storage_proof as v0_8_1_get_storage_proof;
use crate::Starknet;
use crate::StarknetRpcApiError;
use jsonrpsee::core::RpcResult;
use mp_convert::Felt;
use mp_rpc::v0_10_0::BlockId;
use mp_rpc::v0_10_2::{ContractStorageKeysItem, GetStorageProofResult};

pub fn get_storage_proof(
    starknet: &Starknet,
    block_id: BlockId,
    class_hashes: Option<Vec<Felt>>,
    contract_addresses: Option<Vec<Felt>>,
    contracts_storage_keys: Option<Vec<ContractStorageKeysItem>>,
) -> RpcResult<GetStorageProofResult> {
    // Convert StorageKey to Felt for v0.8.1 compatibility
    let contracts_storage_keys_v0_8_1 = convert_storage_keys_for_v0_8_1(contracts_storage_keys)?;
    let block_view = starknet.resolve_view_on(block_id)?;

    Ok(v0_8_1_get_storage_proof::get_storage_proof(
        starknet,
        mp_rpc::v0_8_1::BlockId::Number(block_view.latest_confirmed_block_n().ok_or(StarknetRpcApiError::NoBlocks)?),
        class_hashes,
        contract_addresses,
        contracts_storage_keys_v0_8_1,
    )?)
}
