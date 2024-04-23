use jsonrpsee::core::RpcResult;
use mc_db::storage_handler::{self, StorageView};
use mp_felt::Felt252Wrapper;
use starknet_api::core::{ContractAddress, PatriciaKey};
use starknet_api::hash::StarkFelt;
use starknet_core::types::{BlockId, FieldElement};

use crate::errors::StarknetRpcApiError;
use crate::methods::trace::utils::block_number_by_id;
use crate::Felt;

/// Get the contract class hash in the given block for the contract deployed at the given
/// address
///
/// ### Arguments
///
/// * `block_id` - The hash of the requested block, or number (height) of the requested block, or a
///   block tag
/// * `contract_address` - The address of the contract whose class hash will be returned
///
/// ### Returns
///
/// * `class_hash` - The class hash of the given contract
pub fn get_class_hash_at(block_id: BlockId, contract_address: FieldElement) -> RpcResult<Felt> {
    let block_number = block_number_by_id(block_id);
    let key = ContractAddress(PatriciaKey(StarkFelt(contract_address.to_bytes_be())));

    let Ok(Some(contract_data)) = storage_handler::contract_data().get_at(&key, block_number) else {
        log::error!("Failed to retrieve contract class hash at '{contract_address:?}'");
        return Err(StarknetRpcApiError::ContractNotFound.into());
    };

    Ok(Felt(Felt252Wrapper::from(contract_data.class_hash).into()))
}
