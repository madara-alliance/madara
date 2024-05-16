use jsonrpsee::core::RpcResult;
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
use starknet_core::types::{BlockId, FieldElement, Transaction};

use crate::deoxys_backend_client::get_block_by_block_hash;
use crate::errors::StarknetRpcApiError;
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
pub fn get_transaction_by_block_id_and_index<BE, C, H>(
    starknet: &Starknet<BE, C, H>,
    block_id: BlockId,
    index: u64,
) -> RpcResult<Transaction>
where
    BE: Backend<DBlockT> + 'static,
    C: HeaderBackend<DBlockT> + BlockBackend<DBlockT> + StorageProvider<DBlockT, BE> + 'static,
    C: ProvideRuntimeApi<DBlockT>,
    C::Api: StarknetRuntimeApi<DBlockT> + ConvertTransactionRuntimeApi<DBlockT>,
    H: HasherT + Send + Sync + 'static,
{
    let index = index as usize;
    let substrate_block_hash = starknet.substrate_block_hash_from_starknet_block(block_id)?;

    let starknet_block = get_block_by_block_hash(starknet.client.as_ref(), substrate_block_hash)?;
    let block_number = starknet_block.header().block_number;

    let transaction = starknet_block.transactions().get(index).ok_or(StarknetRpcApiError::InvalidTxnIndex)?;
    let chain_id = starknet.chain_id()?;

    let opt_cached_transaction_hashes =
        starknet.get_cached_transaction_hashes(starknet_block.header().hash::<H>().into());

    // Get the transaction hash from the cache if it exists, otherwise compute it.
    let transaction_hash = match opt_cached_transaction_hashes {
        Some(cached_tx_hashes) => {
            cached_tx_hashes.get(index).map(|&fe| FieldElement::from(Felt252Wrapper::from(fe))).ok_or(
                // This should never happen, because the index is checked above when getting the transaction.
                StarknetRpcApiError::InternalServerError,
            )?
        }
        None => {
            Felt252Wrapper::from(transaction.compute_hash::<H>(chain_id.0.into(), false, Some(block_number)).0).into()
        }
    };

    Ok(to_starknet_core_tx(transaction.clone(), transaction_hash))
}
