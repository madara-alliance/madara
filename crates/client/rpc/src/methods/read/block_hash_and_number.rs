use dp_block::{BlockId, BlockTag};
use starknet_core::types::BlockHashAndNumber;

use crate::{errors::StarknetRpcResult, Starknet};

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
pub fn block_hash_and_number(starknet: &Starknet) -> StarknetRpcResult<BlockHashAndNumber> {
    let block_info = starknet.get_block_info(BlockId::Tag(BlockTag::Latest))?;

    let block_hash = *block_info.block_hash();

    Ok(BlockHashAndNumber { block_hash, block_number: block_info.block_n() })
}
