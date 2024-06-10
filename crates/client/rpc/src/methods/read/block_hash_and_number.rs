use jsonrpsee::core::RpcResult;
use mp_block::{BlockId, BlockTag};
use mp_felt::{FeltWrapper};
use starknet_core::types::BlockHashAndNumber;
use starknet_types_core::felt::Felt;

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


    let block_hash = Felt::from_bytes_be(&block_info.block_hash().0.0);
    let block_hash_as_field = block_hash.into_field_element();

    Ok(BlockHashAndNumber {
        block_hash: block_hash_as_field,
        block_number: block_info.block_n(),
    })
}
