use blockifier::transaction::account_transaction::AccountTransaction;
use jsonrpsee::core::RpcResult;
use mp_felt::Felt252Wrapper;
use mp_hashers::HasherT;
use mp_transactions::TxType;
use mp_types::block::DBlockT;
use pallet_starknet_runtime_api::{ConvertTransactionRuntimeApi, StarknetRuntimeApi};
use sc_client_api::{Backend, BlockBackend, StorageProvider};
use sp_api::ProvideRuntimeApi;
use sp_blockchain::HeaderBackend;
use starknet_api::transaction::Transaction;
use starknet_core::types::{BlockId, TransactionTraceWithHash};

use super::super::read::get_transaction_receipt::execution_infos;
use super::utils::tx_execution_infos_to_tx_trace;
use crate::errors::StarknetRpcApiError;
use crate::madara_backend_client::get_block_by_block_hash;
use crate::utils::helpers::{previous_substrate_block_hash, tx_hash_compute, tx_hash_retrieve};
use crate::utils::transaction::blockifier_transactions;
use crate::Starknet;

pub async fn trace_block_transactions<BE, C, H>(
    starknet: &Starknet<BE, C, H>,
    block_id: BlockId,
) -> RpcResult<Vec<TransactionTraceWithHash>>
where
    BE: Backend<DBlockT> + 'static,
    C: HeaderBackend<DBlockT> + BlockBackend<DBlockT> + StorageProvider<DBlockT, BE> + 'static,
    C: ProvideRuntimeApi<DBlockT>,
    C::Api: StarknetRuntimeApi<DBlockT> + ConvertTransactionRuntimeApi<DBlockT>,
    H: HasherT + Send + Sync + 'static,
{
    let substrate_block_hash = starknet.substrate_block_hash_from_starknet_block(block_id).map_err(|e| {
        log::error!("Block not found: '{e}'");
        StarknetRpcApiError::BlockNotFound
    })?;

    let starknet_block = get_block_by_block_hash(starknet.client.as_ref(), substrate_block_hash).map_err(|e| {
        log::error!("Failed to get block for block hash {substrate_block_hash}: '{e}'");
        StarknetRpcApiError::InternalServerError
    })?;
    let block_header = starknet_block.header().clone();
    let block_number = block_header.block_number;
    let block_hash: Felt252Wrapper = block_header.hash::<H>();
    let previous_block_hash = previous_substrate_block_hash(starknet, substrate_block_hash)?;
    let chain_id = starknet.chain_id()?;

    let block_txs_hashes = if let Some(tx_hashes) = starknet.get_cached_transaction_hashes(block_hash.into()) {
        tx_hash_retrieve(tx_hashes)
    } else {
        tx_hash_compute::<H>(&starknet_block, chain_id)
    };

    let transactions = starknet_block.transactions();
    if transactions.is_empty() {
        log::error!("Failed to retrieve transaction from block with hash {block_hash:?}");
        return Err(StarknetRpcApiError::InternalServerError.into());
    }

    if starknet_block.transactions().iter().any(|transaction| matches!(transaction, Transaction::Deploy(_))) {
        log::error!("Re-executing a deploy transaction is not supported");
        return Err(StarknetRpcApiError::UnimplementedMethod.into());
    }

    let transaction_with_hash =
        starknet_block.transactions().iter().cloned().zip(block_txs_hashes.iter().cloned()).collect();

    let transactions_blockifier = blockifier_transactions(transaction_with_hash)?;

    let fee_token_address = starknet.client.runtime_api().fee_token_addresses(substrate_block_hash).map_err(|e| {
        log::error!("Failed to retrieve fee token address: {e}");
        StarknetRpcApiError::InternalServerError
    })?;

    // TODO(@Tbelleng): check with JB for the good block_contrext
    let block_context =
        block_header.into_block_context(fee_token_address, starknet_api::core::ChainId("SN_MAIN".to_string()));

    let mut transaction_traces = Vec::new();

    for (index, transaction) in transactions_blockifier.iter().enumerate() {
        let transaction_hash = block_txs_hashes[index].clone();

        let tx_type = match transaction {
            blockifier::transaction::transaction_execution::Transaction::AccountTransaction(account_tx) => {
                match account_tx {
                    AccountTransaction::Declare(_) => TxType::Declare,
                    AccountTransaction::DeployAccount(_) => TxType::DeployAccount,
                    AccountTransaction::Invoke(_) => TxType::Invoke,
                }
            }
            blockifier::transaction::transaction_execution::Transaction::L1HandlerTransaction(_) => TxType::L1Handler,
        };

        let execution_infos =
            execution_infos(starknet, previous_block_hash, vec![transaction.clone()], &block_context)?;

        let trace = tx_execution_infos_to_tx_trace(tx_type, &execution_infos, block_number).map_err(|e| {
            log::error!("Failed to generate trace: {}", e);
            StarknetRpcApiError::InternalServerError
        })?;
        let tx_trace = TransactionTraceWithHash { transaction_hash, trace_root: trace };
        transaction_traces.push(tx_trace);
    }

    Ok(transaction_traces)
}
