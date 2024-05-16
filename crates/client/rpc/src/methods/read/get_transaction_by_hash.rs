use jsonrpsee::core::RpcResult;
use mc_db::DeoxysBackend;
use mp_felt::Felt252Wrapper;
use mp_hashers::HasherT;
use mp_transactions::compute_hash::ComputeTransactionHash;
use mp_transactions::to_starknet_core_transaction::to_starknet_core_tx;
use mp_types::block::DBlockT;
use pallet_starknet_runtime_api::{ConvertTransactionRuntimeApi, StarknetRuntimeApi};
use sc_client_api::backend::{Backend, StorageProvider};
use sc_client_api::BlockBackend;
use sp_api::ProvideRuntimeApi;
use sp_blockchain::HeaderBackend;
use starknet_core::types::{FieldElement, Transaction};

use crate::deoxys_backend_client::get_block_by_block_hash;
use crate::errors::StarknetRpcApiError;
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
pub fn get_transaction_by_hash<BE, C, H>(
    starknet: &Starknet<BE, C, H>,
    transaction_hash: FieldElement,
) -> RpcResult<Transaction>
where
    BE: Backend<DBlockT> + 'static,
    C: HeaderBackend<DBlockT> + BlockBackend<DBlockT> + StorageProvider<DBlockT, BE> + 'static,
    C: ProvideRuntimeApi<DBlockT>,
    C::Api: StarknetRuntimeApi<DBlockT> + ConvertTransactionRuntimeApi<DBlockT>,
    H: HasherT + Send + Sync + 'static,
{
    let substrate_block_hash = DeoxysBackend::mapping()
        .block_hash_from_transaction_hash(Felt252Wrapper::from(transaction_hash).into())
        .map_err(|e| {
            log::error!("Failed to get substrate block hash from transaction hash: {}", e);
            StarknetRpcApiError::InternalServerError
        })?
        .ok_or(StarknetRpcApiError::TxnHashNotFound)?;

    let starknet_block = get_block_by_block_hash(starknet.client.as_ref(), substrate_block_hash)?;
    let block_number = starknet_block.header().block_number;
    let starknet_block_hash = starknet_block.header().hash::<H>();

    let chain_id = starknet.chain_id()?.0.into();

    let opt_cached_transaction_hashes = starknet.get_cached_transaction_hashes(starknet_block_hash.into());

    let transaction_hash = Felt252Wrapper::from(transaction_hash);

    let transaction = match opt_cached_transaction_hashes {
        Some(cached_tx_hashes) => cached_tx_hashes
            .into_iter()
            .zip(starknet_block.transactions())
            .find(|(tx_hash, _)| *tx_hash == transaction_hash.into())
            .map(|(_, tx)| tx.clone()),
        None => starknet_block
            .transactions()
            .iter()
            .find(|tx| tx.compute_hash::<H>(chain_id, false, Some(block_number)).0 == transaction_hash.into())
            .cloned(),
    };

    match transaction {
        Some(tx) => Ok(to_starknet_core_tx(tx, transaction_hash.into())),
        // This should never happen, because the transaction hash is checked above when getting block hash from
        // transaction hash.
        None => Err(StarknetRpcApiError::InternalServerError.into()),
    }
}
