use jsonrpsee::core::RpcResult;
use mc_db::storage_handler::primitives::contract_class::{ContractClassWrapper, StorageContractClassData};
use mc_db::storage_handler::{self, StorageView};
use starknet_api::core::{ContractAddress, PatriciaKey};
use starknet_api::hash::StarkFelt;
use starknet_core::types::{BlockId, ContractClass, FieldElement};

use crate::errors::StarknetRpcApiError;
use crate::methods::trace::utils::block_number_by_id;

/// Get the Contract Class Definition at a Given Address in a Specific Block
///
/// ### Arguments
///
/// * `block_id` - The identifier of the block. This can be the hash of the block, its number
///   (height), or a specific block tag.
/// * `contract_address` - The address of the contract whose class definition will be returned.
///
/// ### Returns
///
/// * `contract_class` - The contract class definition. This may be either a standard contract class
///   or a deprecated contract class, depending on the contract's status and the blockchain's
///   version.
///
/// ### Errors
///
/// This method may return the following errors:
/// * `BLOCK_NOT_FOUND` - If the specified block does not exist in the blockchain.
/// * `CONTRACT_NOT_FOUND` - If the specified contract address does not exist.
pub fn get_class_at(block_id: BlockId, contract_address: FieldElement) -> RpcResult<ContractClass> {
    let block_number = block_number_by_id(block_id);
    let key = ContractAddress(PatriciaKey(StarkFelt(contract_address.to_bytes_be())));

    let class_hash = match storage_handler::contract_data().get_class_hash_at(&key, block_number) {
        Err(e) => {
            log::error!("Failed to retrieve contract class: {e}");
            return Err(StarknetRpcApiError::InternalServerError.into());
        }
        Ok(None) => {
            return Err(StarknetRpcApiError::ContractNotFound.into());
        }
        Ok(Some(val)) => val,
    };

    // The class need to be stored
    let Ok(Some(contract_class_data)) = storage_handler::contract_class_data().get(&class_hash) else {
        log::error!("Failed to retrieve contract class from hash: '{}'", class_hash.0);
        return Err(StarknetRpcApiError::InternalServerError.into());
    };

    // converting from stored Blockifier class to rpc class
    let StorageContractClassData { contract_class, abi, sierra_program_length, abi_length } = contract_class_data;
    Ok(ContractClassWrapper { contract: contract_class, abi, sierra_program_length, abi_length }.try_into().map_err(
        |e| {
            log::error!("Failed to convert contract class from hash '{class_hash}' to RPC contract class: {e}");
            StarknetRpcApiError::InternalServerError
        },
    )?)
}
