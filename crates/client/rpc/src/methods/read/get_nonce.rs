use jsonrpsee::core::RpcResult;
use mc_db::storage_handler;
use mp_felt::Felt252Wrapper;
use starknet_api::core::{ContractAddress, PatriciaKey};
use starknet_api::hash::StarkFelt;
use starknet_core::types::{BlockId, FieldElement};

use crate::errors::StarknetRpcApiError;
use crate::methods::trace::utils::block_number_by_id;
use crate::Felt;

/// Get the nonce associated with the given address in the given block.
///
/// ### Arguments
///
/// * `block_id` - The hash of the requested block, or number (height) of the requested block, or a
///   block tag. This parameter specifies the block in which the nonce is to be checked.
/// * `contract_address` - The address of the contract whose nonce we're seeking. This is the unique
///   identifier of the contract in the Starknet network.
///
/// ### Returns
///
/// Returns the contract's nonce at the requested state. The nonce is returned as a
/// `FieldElement`, representing the current state of the contract in terms of transactions
/// count or other contract-specific operations. In case of errors, such as
/// `BLOCK_NOT_FOUND` or `CONTRACT_NOT_FOUND`, returns a `StarknetRpcApiError` indicating the
/// specific issue.
pub fn get_nonce(block_id: BlockId, contract_address: FieldElement) -> RpcResult<Felt> {
    let key = ContractAddress(PatriciaKey(StarkFelt(contract_address.to_bytes_be())));

    let block_number = block_number_by_id(block_id);
    match storage_handler::contract_data().get_nonce_at(&key, block_number) {
        Err(e) => {
            log::error!("Failed to get nonce: {e}");
            Err(StarknetRpcApiError::InternalServerError.into())
        }
        Ok(None) => Err(StarknetRpcApiError::ContractNotFound.into()),
        Ok(Some(nonce)) => Ok(Felt(Felt252Wrapper::from(nonce).into())),
    }
}
