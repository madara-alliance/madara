use jsonrpsee::core::RpcResult;
use mc_db::storage_handler::primitives::contract_class::{ContractClassWrapper, StorageContractClassData};
use mc_db::storage_handler::{self, StorageView};
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

    // TODO: get class for the given block when block_number will be stored in
    // `StorageContractClassData`
    match storage_handler::contract_class_data().get(&class_hash) {
        Err(e) => {
            log::error!("Failed to retrieve contract class: {e}");
            Err(StarknetRpcApiError::InternalServerError.into())
        }
        Ok(None) => Err(StarknetRpcApiError::ClassHashNotFound.into()),
        Ok(Some(class)) => {
            let StorageContractClassData {
                contract_class,
                abi,
                sierra_program_length,
                abi_length,
                block_number: declared_at_block,
            } = class;
            if declared_at_block >= block_number_by_id(block_id)? {
                return Err(StarknetRpcApiError::ClassHashNotFound.into());
            }
            Ok(ContractClassWrapper { contract: contract_class, abi, sierra_program_length, abi_length }
                .try_into()
                .map_err(|e| {
                    log::error!("Failed to convert contract class from hash '{class_hash}' to RPC contract class: {e}");
                    StarknetRpcApiError::InternalServerError
                })?)
        }
    }
}
