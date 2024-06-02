use jsonrpsee::core::RpcResult;
use mc_db::storage_handler::primitives::contract_class::{ContractClassWrapper, StorageContractClassData};
use mc_db::storage_handler::{self, StorageView};
use mp_convert::field_element::FromFieldElement;
use starknet_api::core::ClassHash;
use starknet_core::types::{BlockId, ContractClass, FieldElement};

use crate::errors::StarknetRpcApiError;
use crate::utils::ResultExt;
use crate::Starknet;

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
pub fn get_class(starknet: &Starknet, block_id: BlockId, class_hash: FieldElement) -> RpcResult<ContractClass> {
    let class_hash = ClassHash::from_field_element(class_hash);

    // TODO: get class for the given block when block_number will be stored in
    // `StorageContractClassData`

    let class = storage_handler::contract_class_data()
        .get(&class_hash)
        .or_internal_server_error("Failed to retrieve contract class")?
        .ok_or(StarknetRpcApiError::ClassHashNotFound)?;

    let StorageContractClassData {
        contract_class,
        abi,
        sierra_program_length,
        abi_length,
        block_number: declared_at_block,
    } = class;

    if declared_at_block >= starknet.get_block_n(block_id)? {
        return Err(StarknetRpcApiError::ClassHashNotFound.into());
    }
    Ok(ContractClassWrapper { contract: contract_class, abi, sierra_program_length, abi_length }
        .try_into()
        .or_else_internal_server_error(|| {
            format!("Failed to convert contract class from hash '{class_hash}' to RPC contract class")
        })?)
}
