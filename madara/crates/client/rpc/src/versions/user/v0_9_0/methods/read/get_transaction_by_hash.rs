use crate::errors::{StarknetRpcApiError, StarknetRpcResult};
use crate::Starknet;
use mp_rpc::v0_9_0::TxnWithHash;
use starknet_types_core::felt::Felt;

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
/// - `TXN_HASH_NOT_FOUND` if the specified transaction is not found.
pub fn get_transaction_by_hash(starknet: &Starknet, transaction_hash: Felt) -> StarknetRpcResult<TxnWithHash> {
    let view = starknet.backend.view_on_latest();
    let res = view.find_transaction_by_hash(&transaction_hash)?.ok_or(StarknetRpcApiError::TxnHashNotFound)?;
    let transaction = res.get_transaction()?;
    Ok(TxnWithHash { transaction: transaction.transaction.to_rpc_v0_8(), transaction_hash })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{sample_chain_for_block_getters, SampleChainForBlockGetters};
    use rstest::rstest;

    #[rstest]
    fn test_get_transaction_by_hash(sample_chain_for_block_getters: (SampleChainForBlockGetters, Starknet)) {
        let (SampleChainForBlockGetters { tx_hashes, expected_txs_v0_8, .. }, rpc) = sample_chain_for_block_getters;

        // Block 0
        assert_eq!(get_transaction_by_hash(&rpc, tx_hashes[0]).unwrap(), expected_txs_v0_8[0]);

        // Block 1

        // Block 2
        assert_eq!(get_transaction_by_hash(&rpc, tx_hashes[1]).unwrap(), expected_txs_v0_8[1]);
        assert_eq!(get_transaction_by_hash(&rpc, tx_hashes[2]).unwrap(), expected_txs_v0_8[2]);

        // Pending
        assert_eq!(get_transaction_by_hash(&rpc, tx_hashes[3]).unwrap(), expected_txs_v0_8[3]);
    }

    #[rstest]
    fn test_get_transaction_by_hash_not_found(sample_chain_for_block_getters: (SampleChainForBlockGetters, Starknet)) {
        let (SampleChainForBlockGetters { .. }, rpc) = sample_chain_for_block_getters;

        let does_not_exist = Felt::from_hex_unchecked("0x7128638126378");
        assert_eq!(get_transaction_by_hash(&rpc, does_not_exist), Err(StarknetRpcApiError::TxnHashNotFound));
    }
}
