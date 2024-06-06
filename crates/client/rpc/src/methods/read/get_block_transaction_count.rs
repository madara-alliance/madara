use jsonrpsee::core::RpcResult;
use starknet_core::types::BlockId;

use crate::Starknet;

/// Get the Number of Transactions in a Given Block
///
/// ### Arguments
///
/// * `block_id` - The identifier of the requested block. This can be the hash of the block, the
///   block's number (height), or a specific block tag.
///
/// ### Returns
///
/// * `transaction_count` - The number of transactions in the specified block.
///
/// ### Errors
///
/// This function may return a `BLOCK_NOT_FOUND` error if the specified block does not exist in
/// the blockchain.
pub fn get_block_transaction_count(starknet: &Starknet, block_id: BlockId) -> RpcResult<u128> {
    let block = starknet.get_block_info(block_id)?;

    Ok(block.header().transaction_count)
}
