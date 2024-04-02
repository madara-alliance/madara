use deoxys_runtime::opaque::DBlockT;
use jsonrpsee::core::RpcResult;
use mc_db::DeoxysBackend;
use mc_genesis_data_provider::GenesisProvider;
use mp_felt::Felt252Wrapper;
use mp_hashers::HasherT;
use pallet_starknet_runtime_api::{ConvertTransactionRuntimeApi, StarknetRuntimeApi};
use sc_client_api::{Backend, BlockBackend, StorageProvider};
use sc_transaction_pool::ChainApi;
use sc_transaction_pool_api::TransactionPool;
use sp_api::ProvideRuntimeApi;
use sp_blockchain::HeaderBackend;
use starknet_core::types::TransactionTraceWithHash;
use starknet_ff::FieldElement;

use super::utils::{
    get_previous_block_substrate_hash, map_transaction_to_user_transaction, tx_execution_infos_to_tx_trace,
};
use crate::errors::StarknetRpcApiError;
use crate::utils::get_block_by_block_hash;
use crate::{Starknet, StarknetReadRpcApiServer};

pub async fn trace_transaction<A, BE, G, C, P, H>(
    starknet: &Starknet<A, BE, G, C, P, H>,
    transaction_hash: FieldElement,
) -> RpcResult<TransactionTraceWithHash>
where
    A: ChainApi<Block = DBlockT> + 'static,
    BE: Backend<DBlockT> + 'static,
    G: GenesisProvider + Send + Sync + 'static,
    C: HeaderBackend<DBlockT> + BlockBackend<DBlockT> + StorageProvider<DBlockT, BE> + 'static,
    C: ProvideRuntimeApi<DBlockT>,
    C::Api: StarknetRuntimeApi<DBlockT> + ConvertTransactionRuntimeApi<DBlockT>,
    P: TransactionPool<Block = DBlockT> + 'static,
    H: HasherT + Send + Sync + 'static,
{
    let substrate_block_hash = DeoxysBackend::mapping()
        .block_hash_from_transaction_hash(Felt252Wrapper(transaction_hash).into())
        .map_err(|e| {
            log::error!("Failed to get transaction's substrate block hash from mapping_db: {e}");
            StarknetRpcApiError::TxnHashNotFound
        })?
        .ok_or(StarknetRpcApiError::TxnHashNotFound)?;

    let starknet_block = get_block_by_block_hash(starknet.client.as_ref(), substrate_block_hash)?;
    let block_header = starknet_block.header().clone();
    let chain_id = Felt252Wrapper(starknet.chain_id()?.0);
    let transaction_hash_to_trace: Felt252Wrapper = transaction_hash.into();

    let (txs_to_execute_before, tx_to_trace) = map_transaction_to_user_transaction(
        starknet,
        starknet_block,
        substrate_block_hash,
        chain_id,
        Some(transaction_hash_to_trace),
    )?;

    let previous_block_substrate_hash = get_previous_block_substrate_hash(starknet, substrate_block_hash)?;

    let fee_token_address = starknet.client.runtime_api().fee_token_addresses(substrate_block_hash).map_err(|e| {
        log::error!("Failed to retrieve fee token address");
        StarknetRpcApiError::InternalServerError
    })?;
    // TODO: convert the real chain_id in String
    let block_context =
        block_header.into_block_context(fee_token_address, starknet_api::core::ChainId("SN_MAIN".to_string()));

    let execution_infos = starknet
        .client
        .runtime_api()
        .re_execute_transactions(
            previous_block_substrate_hash,
            txs_to_execute_before.clone(),
            tx_to_trace.clone(),
            &block_context,
        )
        .map_err(|e| {
            log::error!("Failed to execute runtime API call: {e}");
            StarknetRpcApiError::InternalServerError
        })?
        .map_err(|e| {
            log::error!("Failed to reexecute the block transactions: {e:?}");
            StarknetRpcApiError::InternalServerError
        })?
        .map_err(|_| {
            log::error!(
                "One of the transaction failed during it's reexecution. This should not happen, as the block has \
                 already been executed successfully in the past. There is a bug somewhere."
            );
            StarknetRpcApiError::InternalServerError
        })?;

    let storage_override = starknet.overrides.for_block_hash(starknet.client.as_ref(), substrate_block_hash);
    let _chain_id = Felt252Wrapper(starknet.chain_id()?.0);

    let trace = tx_execution_infos_to_tx_trace(
        &**storage_override,
        substrate_block_hash,
        tx_to_trace.get(0).unwrap().tx_type(),
        &execution_infos[0],
    )
    .unwrap();

    let tx_trace = TransactionTraceWithHash { transaction_hash, trace_root: trace };

    Ok(tx_trace)
}
