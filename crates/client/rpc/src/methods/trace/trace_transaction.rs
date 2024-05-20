use blockifier::transaction::account_transaction::AccountTransaction;
use jsonrpsee::core::RpcResult;
use mc_db::DeoxysBackend;
use mp_felt::Felt252Wrapper;
use mp_hashers::HasherT;
use mp_transactions::TxType;
use mp_types::block::DBlockT;
use pallet_starknet_runtime_api::StarknetRuntimeApi;
use sc_client_api::{Backend, BlockBackend, StorageProvider};
use sp_api::ProvideRuntimeApi;
use sp_blockchain::HeaderBackend;
use starknet_api::transaction::Transaction;
use starknet_core::types::TransactionTraceWithHash;
use starknet_ff::FieldElement;

use super::super::read::get_transaction_receipt::execution_infos;
use super::utils::tx_execution_infos_to_tx_trace;
use crate::deoxys_backend_client::get_block_by_block_hash;
use crate::errors::StarknetRpcApiError;
use crate::utils::execution::block_context;
use crate::utils::helpers::{tx_hash_compute, tx_hash_retrieve};
use crate::utils::transaction::blockifier_transactions;
use crate::Starknet;

pub async fn trace_transaction<BE, C, H>(
    starknet: &Starknet<BE, C, H>,
    transaction_hash: FieldElement,
) -> RpcResult<TransactionTraceWithHash>
where
    BE: Backend<DBlockT> + 'static,
    C: HeaderBackend<DBlockT> + BlockBackend<DBlockT> + StorageProvider<DBlockT, BE> + 'static,
    C: ProvideRuntimeApi<DBlockT>,
    C::Api: StarknetRuntimeApi<DBlockT>,
    H: HasherT + Send + Sync + 'static,
{
    let substrate_block_hash = DeoxysBackend::mapping()
        .block_hash_from_transaction_hash(Felt252Wrapper(transaction_hash).into())
        .map_err(|e| {
            log::error!("Failed to get substrate block hash from transaction hash: {}", e);
            StarknetRpcApiError::TxnHashNotFound
        })?
        .ok_or(StarknetRpcApiError::TxnHashNotFound)?;

    let starknet_block = get_block_by_block_hash(starknet.client.as_ref(), substrate_block_hash)?;
    let block_header = starknet_block.header();
    let block_hash: Felt252Wrapper = block_header.hash::<H>();
    let block_number = block_header.block_number;
    let chain_id = starknet.chain_id()?;
    let block_context = block_context(starknet.client.as_ref(), substrate_block_hash)?;

    // retrieve all transaction hashes from the block in the cache or compute them
    // here we can optimize by computing only tx before the one that we want to trace (optimized if we
    // dont use cache)
    let block_txs_hashes = if let Some(tx_hashes) = starknet.get_cached_transaction_hashes(block_hash.into()) {
        tx_hash_retrieve(tx_hashes)
    } else {
        tx_hash_compute::<H>(&starknet_block, chain_id)
    };

    // retrieve the transaction index in the block with the transaction hash
    let (tx_index, _) =
        block_txs_hashes.iter().enumerate().find(|(_, hash)| *hash == &transaction_hash).ok_or_else(|| {
            log::error!("Failed to retrieve transaction index from block with hash {block_hash:?}");
            StarknetRpcApiError::InternalServerError
        })?;

    // create a vector of tuples with the transaction and its hash, up to the current transaction index
    let transaction_with_hash = starknet_block
        .transactions()
        .iter()
        .cloned()
        .zip(block_txs_hashes.iter().cloned())
        .filter(|(tx, _)| !matches!(tx, Transaction::Deploy(_)))
        .take(tx_index + 1)
        .collect();

    let transactions_blockifier = blockifier_transactions(transaction_with_hash)?;

    let last_transaction = transactions_blockifier.last().expect("There should be at least one transaction");

    let tx_type = match last_transaction {
        blockifier::transaction::transaction_execution::Transaction::AccountTransaction(account_tx) => match account_tx
        {
            AccountTransaction::Declare(_) => TxType::Declare,
            AccountTransaction::DeployAccount(_) => TxType::DeployAccount,
            AccountTransaction::Invoke(_) => TxType::Invoke,
        },
        blockifier::transaction::transaction_execution::Transaction::L1HandlerTransaction(_) => TxType::L1Handler,
    };

    let execution_infos = execution_infos(transactions_blockifier, &block_context)?;

    let trace = tx_execution_infos_to_tx_trace(tx_type, &execution_infos, block_number).unwrap();

    let tx_trace = TransactionTraceWithHash { transaction_hash, trace_root: trace };

    Ok(tx_trace)
}
