use jsonrpsee::core::RpcResult;
use mc_db::DeoxysBackend;
use mp_felt::Felt252Wrapper;
use mp_hashers::HasherT;
use mp_transactions::TxType;
use mp_types::block::DBlockT;
use pallet_starknet_runtime_api::{ConvertTransactionRuntimeApi, StarknetRuntimeApi};
use sc_client_api::{Backend, BlockBackend, StorageProvider};
use sp_api::ProvideRuntimeApi;
use sp_blockchain::HeaderBackend;
use starknet_core::types::TransactionTraceWithHash;
use starknet_ff::FieldElement;

use super::utils::{map_transaction_to_user_transaction, tx_execution_infos_to_tx_trace};
use crate::errors::StarknetRpcApiError;
use crate::madara_backend_client::get_block_by_block_hash;
use crate::utils::execution::re_execute_transactions;
use crate::Starknet;

pub async fn trace_transaction<BE, C, H>(
    starknet: &Starknet<BE, C, H>,
    transaction_hash: FieldElement,
) -> RpcResult<TransactionTraceWithHash>
where
    BE: Backend<DBlockT> + 'static,
    C: HeaderBackend<DBlockT> + BlockBackend<DBlockT> + StorageProvider<DBlockT, BE> + 'static,
    C: ProvideRuntimeApi<DBlockT>,
    C::Api: StarknetRuntimeApi<DBlockT> + ConvertTransactionRuntimeApi<DBlockT>,
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
    let block_number = block_header.block_number;
    let chain_id = Felt252Wrapper(starknet.chain_id()?.0);
    let transaction_hash_to_trace: Felt252Wrapper = transaction_hash.into();

    let (txs_to_execute_before, tx_to_trace) =
        map_transaction_to_user_transaction::<H>(starknet_block, chain_id, Some(transaction_hash_to_trace))?;

    let fee_token_address = starknet.client.runtime_api().fee_token_addresses(substrate_block_hash).map_err(|e| {
        log::error!("Failed to retrieve fee token address: {e}");
        StarknetRpcApiError::InternalServerError
    })?;
    // TODO: convert the real chain_id in String
    let block_context =
        block_header.into_block_context(fee_token_address, starknet_api::core::ChainId("SN_MAIN".to_string()));

    let execution_infos = re_execute_transactions(txs_to_execute_before.clone(), tx_to_trace.clone(), &block_context)
        .map_err(|e| {
        log::error!("Failed to reexecute the block transactions: {e:?}");
        StarknetRpcApiError::InternalServerError
    })?;

    let trace =
        tx_execution_infos_to_tx_trace(TxType::from(tx_to_trace.get(0).unwrap()), &execution_infos[0], block_number)
            .unwrap();

    let tx_trace = TransactionTraceWithHash { transaction_hash, trace_root: trace };

    Ok(tx_trace)
}
