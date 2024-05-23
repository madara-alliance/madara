use jsonrpsee::core::RpcResult;
use mp_felt::Felt252Wrapper;
use mp_hashers::HasherT;
use mp_transactions::TxType;
use mp_types::block::DBlockT;
use sc_client_api::{Backend, BlockBackend, StorageProvider};
use sp_blockchain::HeaderBackend;
use starknet_api::transaction::Transaction;
use starknet_core::types::{BlockId, TransactionTraceWithHash};

use super::utils::tx_execution_infos_to_tx_trace;
use crate::deoxys_backend_client::get_block_by_block_hash;
use crate::errors::StarknetRpcApiError;
use crate::utils::execution::{block_context, re_execute_transactions};
use crate::utils::helpers::tx_hash_retrieve;
use crate::utils::transaction::blockifier_transactions;
use crate::Starknet;

pub async fn trace_block_transactions<BE, C, H>(
    starknet: &Starknet<BE, C, H>,
    block_id: BlockId,
) -> RpcResult<Vec<TransactionTraceWithHash>>
where
    BE: Backend<DBlockT> + 'static,
    C: HeaderBackend<DBlockT> + BlockBackend<DBlockT> + StorageProvider<DBlockT, BE> + 'static,
    H: HasherT + Send + Sync + 'static,
{
    let substrate_block_hash = starknet.substrate_block_hash_from_starknet_block(block_id)?;

    let starknet_block = get_block_by_block_hash(starknet.client.as_ref(), substrate_block_hash).map_err(|e| {
        log::error!("Failed to get block for block hash {substrate_block_hash}: '{e}'");
        StarknetRpcApiError::InternalServerError
    })?;
    let block_header = starknet_block.header();
    let block_number = block_header.block_number;
    let block_hash: Felt252Wrapper = block_header.hash::<H>();
    let block_context = block_context(starknet.client.as_ref(), substrate_block_hash)?;

    let block_txs_hashes = tx_hash_retrieve(starknet.get_cached_transaction_hashes(block_hash.into())?);

    // create a vector of transactions with their corresponding hashes without deploy transactions,
    // blockifier does not support deploy transactions
    let transaction_with_hash: Vec<_> = starknet_block
        .transactions()
        .iter()
        .cloned()
        .zip(block_txs_hashes.iter().cloned())
        .filter(|(tx, _)| !matches!(tx, Transaction::Deploy(_)))
        .collect();

    let transactions_blockifier = blockifier_transactions(transaction_with_hash.clone())?;

    let mut transactions_traces = Vec::new();

    let execution_infos = re_execute_transactions(vec![], transactions_blockifier, &block_context).map_err(|e| {
        log::error!("Failed to re-execute transactions: '{e}'");
        StarknetRpcApiError::InternalServerError
    })?;

    for (index, (transaction, tx_hash)) in transaction_with_hash.iter().enumerate() {
        let tx_type = match transaction {
            Transaction::Declare(_) => TxType::Declare,
            Transaction::DeployAccount(_) => TxType::DeployAccount,
            Transaction::Invoke(_) => TxType::Invoke,
            Transaction::L1Handler(_) => TxType::L1Handler,
            Transaction::Deploy(_) => unreachable!(),
        };

        match tx_execution_infos_to_tx_trace(tx_type, &execution_infos[index], block_number) {
            Ok(trace) => {
                let transaction_trace = TransactionTraceWithHash { trace_root: trace, transaction_hash: *tx_hash };
                transactions_traces.push(transaction_trace);
            }
            Err(e) => {
                log::error!("Failed to generate trace: {}", e);
                return Err(StarknetRpcApiError::InternalServerError.into());
            }
        }
    }

    Ok(transactions_traces)
}
