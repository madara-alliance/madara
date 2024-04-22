use jsonrpsee::core::RpcResult;
use mc_db::storage_handler::{self, StorageView};
use mp_contract::class::ContractClassWrapper;
use mp_felt::Felt252Wrapper;
use starknet_core::types::{BlockId, ContractClass, FieldElement};

use crate::errors::StarknetRpcApiError;
use crate::methods::trace::utils::block_number_by_id;

/// Get the contract class definition in the given block associated with the given hash.
///
/// ### Arguments
///
/// * `block_id` - The hash of the requested block, or number (height) of the requested block, or a
///   block tag.
/// * `class_hash` - The hash of the requested contract class.
///
/// ### Returns
///
/// Returns the contract class definition if found. In case of an error, returns a
/// `StarknetRpcApiError` indicating either `BlockNotFound` or `ClassHashNotFound`.
pub fn get_class(block_id: BlockId, class_hash: FieldElement) -> RpcResult<ContractClass> {
    let class_hash = Felt252Wrapper(class_hash).into();

    let block_number = block_number_by_id(block_id);

    let Ok(Some(contract_class)) = storage_handler::contract_class().get_at(&class_hash, block_number) else {
        log::error!("Failed to retrieve contract class from hash '{class_hash}'");
        return Err(StarknetRpcApiError::ClassHashNotFound.into());
    };

    let Ok(Some(contract_abi)) = storage_handler::contract_abi().get_at(&class_hash, block_number) else {
        log::error!("Failed to retrieve contract ABI from hash '{class_hash}'");
        return Err(StarknetRpcApiError::ClassHashNotFound.into());
    };

    // converting from stored Blockifier class to rpc class
    Ok(ContractClassWrapper { contract: contract_class, abi: contract_abi }.try_into().map_err(|e| {
        log::error!("Failed to convert contract class from hash '{class_hash}' to RPC contract class: {e}");
        StarknetRpcApiError::InternalServerError
    })?)
}
