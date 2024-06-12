use dp_convert::field_element::FromFieldElement;
use dp_felt::FeltWrapper;
use jsonrpsee::core::RpcResult;
use starknet_api::core::ContractAddress;
use starknet_core::types::{BlockId, FieldElement};

use crate::errors::StarknetRpcApiError;
use crate::utils::ResultExt;
use crate::{Felt, Starknet};

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
pub fn get_nonce(starknet: &Starknet, block_id: BlockId, contract_address: FieldElement) -> RpcResult<Felt> {
    let block_number = starknet.get_block_n(block_id)?;
    let key = ContractAddress::from_field_element(contract_address);
    let felt = starknet.backend.contract_nonces()
        .get_at(&key, block_number)
        .or_internal_server_error("Failed to retrieve contract class")?
        .ok_or(StarknetRpcApiError::ContractNotFound)?;

    Ok(Felt(felt.into_field_element()))
}
