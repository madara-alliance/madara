use dp_transactions::to_starknet_core_transaction::to_starknet_core_tx;
use jsonrpsee::core::RpcResult;
use starknet_core::types::{Felt, Transaction};

use crate::errors::StarknetRpcApiError;
use crate::utils::{OptionExt, ResultExt};
use crate::Starknet;

/// Get the details and status of a submitted transaction.
///
/// This function retrieves the detailed information and status of a transaction identified by
/// its hash. The transaction hash uniquely identifies a specific transaction that has been
/// submitted to the StarkNet network.
///
/// ### Arguments
///
/// * `transaction_hash` - The hash of the requested transaction. This parameter specifies the
///   transaction for which details and status are requested.
///
/// ### Returns
///
/// Returns information about the requested transaction, including its status, sender,
/// recipient, and other transaction details. The information is encapsulated in a `Transaction`
/// type, which is a combination of the `TXN` schema and additional properties, such as the
/// `transaction_hash`. In case the specified transaction hash is not found, returns a
/// `StarknetRpcApiError` with `TXN_HASH_NOT_FOUND`.
///
/// ### Errors
///
/// The function may return one of the following errors if encountered:
/// - `PAGE_SIZE_TOO_BIG` if the requested page size exceeds the allowed limit.
/// - `INVALID_CONTINUATION_TOKEN` if the provided continuation token is invalid or expired.
/// - `BLOCK_NOT_FOUND` if the specified block is not found.
/// - `TOO_MANY_KEYS_IN_FILTER` if there are too many keys in the filter, which may exceed the
///   system's capacity.
pub fn get_transaction_by_hash(starknet: &Starknet, transaction_hash: Felt) -> RpcResult<Transaction> {
    let (block, tx_info) = starknet
        .block_storage()
        .find_tx_hash_block(&transaction_hash)
        .or_internal_server_error("Error getting block from tx hash")?
        .ok_or(StarknetRpcApiError::TxnHashNotFound)?;
    let tx =
        block.transactions().get(tx_info.tx_index).ok_or_internal_server_error("Storage block transaction mismatch")?;
    Ok(to_starknet_core_tx(tx, transaction_hash))
}
