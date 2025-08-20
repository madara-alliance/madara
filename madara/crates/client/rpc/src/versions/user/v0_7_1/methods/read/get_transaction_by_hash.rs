use mp_rpc::v0_7_1::TxnWithHash;
use starknet_types_core::felt::Felt;

use crate::errors::{StarknetRpcApiError, StarknetRpcResult};
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
pub fn get_transaction_by_hash(starknet: &Starknet, transaction_hash: Felt) -> StarknetRpcResult<TxnWithHash> {
    let (block, tx_index) = starknet
        .backend
        .find_tx_hash_block(&transaction_hash)
        .or_internal_server_error("Error getting block from tx hash")?
        .ok_or(StarknetRpcApiError::TxnHashNotFound)?;
    let transaction = block
        .inner
        .transactions
        .into_iter()
        .nth(tx_index.0 as usize)
        .ok_or_internal_server_error("Storage block transaction mismatch")?;

    Ok(TxnWithHash { transaction: transaction.into(), transaction_hash })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{sample_chain_for_block_getters, SampleChainForBlockGetters};
    use rstest::rstest;

    #[rstest]
    fn test_get_transaction_by_hash(sample_chain_for_block_getters: (SampleChainForBlockGetters, Starknet)) {
        let (SampleChainForBlockGetters { tx_hashes, expected_txs, .. }, rpc) = sample_chain_for_block_getters;

        // Block 0
        assert_eq!(get_transaction_by_hash(&rpc, tx_hashes[0]).unwrap(), expected_txs[0]);

        // Block 1

        // Block 2
        assert_eq!(get_transaction_by_hash(&rpc, tx_hashes[1]).unwrap(), expected_txs[1]);
        assert_eq!(get_transaction_by_hash(&rpc, tx_hashes[2]).unwrap(), expected_txs[2]);

        // Pending
        assert_eq!(get_transaction_by_hash(&rpc, tx_hashes[3]).unwrap(), expected_txs[3]);
    }

    #[rstest]
    fn test_get_transaction_by_hash_not_found(sample_chain_for_block_getters: (SampleChainForBlockGetters, Starknet)) {
        let (SampleChainForBlockGetters { .. }, rpc) = sample_chain_for_block_getters;

        let does_not_exist = Felt::from_hex_unchecked("0x7128638126378");
        assert_eq!(get_transaction_by_hash(&rpc, does_not_exist), Err(StarknetRpcApiError::TxnHashNotFound));
    }
}
