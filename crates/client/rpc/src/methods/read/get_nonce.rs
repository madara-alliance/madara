use dp_convert::ToFelt;
use dp_convert::ToStarkFelt;
use jsonrpsee::core::RpcResult;
use starknet_core::types::BlockId;
use starknet_types_core::felt::Felt;

use crate::errors::StarknetRpcApiError;
use crate::utils::ResultExt;
use crate::Starknet;

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
/// `Felt`, representing the current state of the contract in terms of transactions
/// count or other contract-specific operations. In case of errors, such as
/// `BLOCK_NOT_FOUND` or `CONTRACT_NOT_FOUND`, returns a `StarknetRpcApiError` indicating the
/// specific issue.
pub fn get_nonce(starknet: &Starknet, block_id: BlockId, contract_address: Felt) -> RpcResult<Felt> {
    let block_number = starknet.get_block_n(block_id)?;
    let key = contract_address.to_stark_felt().try_into().map_err(StarknetRpcApiError::from)?;
    let nonce = starknet
        .backend
        .contract_nonces()
        .get_at(&key, block_number)
        .or_internal_server_error("Failed to retrieve contract class")?
        .ok_or(StarknetRpcApiError::ContractNotFound)?;

    Ok(nonce.to_felt())
}
