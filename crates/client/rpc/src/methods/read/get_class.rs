use jsonrpsee::core::RpcResult;
use mc_db::storage_handler::{self, StorageView};
use mp_contract::class::{ContractClassWrapper, StorageContractClassData};
use mp_felt::Felt252Wrapper;
use starknet_core::types::{BlockId, ContractClass, FieldElement};

use crate::errors::StarknetRpcApiError;

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
pub fn get_class(_block_id: BlockId, class_hash: FieldElement) -> RpcResult<ContractClass> {
    let class_hash = Felt252Wrapper(class_hash).into();

    // TODO: is it ok to ignore `block_id` in this case? IE: is `contract_class` revertible on the
    // chain? @charpao what are your thoughts?

    let Ok(Some(contract_class_data)) = storage_handler::contract_class_data().get(&class_hash) else {
        log::error!("Failed to retrieve contract class from hash '{class_hash}'");
        return Err(StarknetRpcApiError::ClassHashNotFound.into());
    };

    // converting from stored Blockifier class to rpc class
    // TODO: retrieve sierra_program_length and abi_length when they are stored in the storage
    let StorageContractClassData { contract_class, abi } = contract_class_data;
    Ok(ContractClassWrapper { contract: contract_class, abi, sierra_program_length: 0, abi_length: 0 }
        .try_into()
        .map_err(|e| {
            log::error!("Failed to convert contract class from hash '{class_hash}' to RPC contract class: {e}");
            StarknetRpcApiError::InternalServerError
        })?)
}
