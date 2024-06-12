use dp_block::{BlockId, BlockTag};
use dp_felt::Felt252Wrapper;
use jsonrpsee::core::RpcResult;
use starknet_core::types::BlockHashAndNumber;

use crate::Starknet;

/// Get the Most Recent Accepted Block Hash and Number
///
/// ### Arguments
///
/// This function does not take any arguments.
///
/// ### Returns
///
/// * `block_hash_and_number` - A tuple containing the latest block hash and number of the current
///   network.
pub fn block_hash_and_number(starknet: &Starknet) -> RpcResult<BlockHashAndNumber> {
    let block_info = starknet.get_block_info(BlockId::Tag(BlockTag::Latest))?;

    Ok(BlockHashAndNumber {
        block_hash: Felt252Wrapper::from(block_info.block_hash().0).0,
        block_number: block_info.block_n(),
    })
}
