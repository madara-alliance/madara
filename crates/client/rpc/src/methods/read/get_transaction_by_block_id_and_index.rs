use starknet_core::types::{BlockId, Transaction};

use crate::errors::{StarknetRpcApiError, StarknetRpcResult};
use crate::Starknet;

/// Get the details of a transaction by a given block id and index.
///
/// This function fetches the details of a specific transaction in the StarkNet network by
/// identifying it through its block and position (index) within that block. If no transaction
/// is found at the specified index, null is returned.
///
/// ### Arguments
///
/// * `block_id` - The hash of the requested block, or number (height) of the requested block, or a
///   block tag. This parameter is used to specify the block in which the transaction is located.
/// * `index` - An integer representing the index in the block where the transaction is expected to
///   be found. The index starts from 0 and increases sequentially for each transaction in the
///   block.
///
/// ### Returns
///
/// Returns the details of the transaction if found, including the transaction hash. The
/// transaction details are returned as a type conforming to the StarkNet protocol. In case of
/// errors like `BLOCK_NOT_FOUND` or `INVALID_TXN_INDEX`, returns a `StarknetRpcApiError`
/// indicating the specific issue.
pub fn get_transaction_by_block_id_and_index(
    starknet: &Starknet,
    block_id: BlockId,
    index: u64,
) -> StarknetRpcResult<Transaction> {
    let block = starknet.get_block(&block_id)?;
    let transaction_hash = block.info.tx_hashes().get(index as usize).ok_or(StarknetRpcApiError::InvalidTxnIndex)?;
    let transaction = block.inner.transactions.get(index as usize).ok_or(StarknetRpcApiError::InvalidTxnIndex)?;

    Ok(transaction.clone().to_core(*transaction_hash))
}
