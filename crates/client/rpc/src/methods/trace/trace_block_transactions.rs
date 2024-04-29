use jsonrpsee::core::RpcResult;
use mp_felt::Felt252Wrapper;
use mp_hashers::HasherT;
use mp_transactions::getters::Hash;
use mp_transactions::TxType;
use mp_types::block::DBlockT;
use pallet_starknet_runtime_api::{ConvertTransactionRuntimeApi, StarknetRuntimeApi};
use sc_client_api::{Backend, BlockBackend, StorageProvider};
use sp_api::ProvideRuntimeApi;
use sp_blockchain::HeaderBackend;
use starknet_core::types::{BlockId, TransactionTraceWithHash};

use super::utils::{map_transaction_to_user_transaction, tx_execution_infos_to_tx_trace};
use crate::deoxys_backend_client::get_block_by_block_hash;
use crate::errors::StarknetRpcApiError;
use crate::methods::trace::utils::block_number_by_id;
use crate::utils::execution::re_execute_transactions;
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
    let chain_id = Felt252Wrapper(starknet.chain_id()?.0);

    let (block_transactions, empty_transactions) =
        map_transaction_to_user_transaction::<H>(starknet_block, chain_id, None)?;

    let fee_token_address = starknet.client.runtime_api().fee_token_addresses(substrate_block_hash).map_err(|e| {
        log::error!("Failed to retrieve fee token address: '{e}'");
        StarknetRpcApiError::InternalServerError
    })?;
    let block = get_block_by_block_hash(starknet.client.as_ref(), substrate_block_hash)?;
    let block_header = block.header();
    // TODO: convert the real chain_id in String
    let block_context =
        block_header.into_block_context(fee_token_address, starknet_api::core::ChainId("SN_MAIN".to_string()));

    let execution_infos =
        re_execute_transactions(empty_transactions.clone(), block_transactions.clone(), &block_context).map_err(
            |e| {
                log::error!("Failed to reexecute the block transactions: {e:?}");
                StarknetRpcApiError::InternalServerError
            },
        )?;

    let block_number = block_number_by_id(block_id);
    let traces = execution_infos
        .into_iter()
        .enumerate()
        .map(|(tx_idx, tx_exec_info)| {
            tx_execution_infos_to_tx_trace(
                // Safe to unwrap coz re_execute returns exactly one ExecutionInfo for each tx
                TxType::from(block_transactions.get(tx_idx).unwrap()),
                &tx_exec_info,
                block_number,
            )
            .map(|trace_root| TransactionTraceWithHash {
                transaction_hash: Felt252Wrapper::from(block_transactions[tx_idx].tx_hash().unwrap()).into(),
                trace_root,
            })
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(StarknetRpcApiError::from)?;

    Ok(traces)
}
