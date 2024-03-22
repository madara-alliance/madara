use blockifier::transaction::objects::TransactionExecutionInfo;
use jsonrpsee::core::RpcResult;
use log::error;
use mc_genesis_data_provider::GenesisProvider;
use mc_storage::StorageOverride;
use mp_felt::Felt252Wrapper;
use mp_hashers::HasherT;
use mp_simulations::{PlaceHolderErrorTypeForFailedStarknetExecution, SimulationFlags};
use mp_transactions::TxType;
use pallet_starknet_runtime_api::{ConvertTransactionRuntimeApi, StarknetRuntimeApi};
use sc_client_api::{Backend, BlockBackend, StorageProvider};
use sc_transaction_pool::ChainApi;
use sc_transaction_pool_api::TransactionPool;
use sp_api::ProvideRuntimeApi;
use sp_blockchain::HeaderBackend;
use sp_runtime::traits::Block as BlockT;
use starknet_core::types::{BlockId, BroadcastedTransaction, FeeEstimate, SimulatedTransaction, SimulationFlag};

use super::lib::ConvertCallInfoToExecuteInvocationError;
use super::trace_implementation::tx_execution_infos_to_tx_trace;
use crate::errors::StarknetRpcApiError;
use crate::{Starknet, StarknetReadRpcApiServer};

#[allow(unused_variables)]
pub async fn simulate_transactions<A, B, BE, G, C, P, H>(
    starknet: &Starknet<A, B, BE, G, C, P, H>,
    block_id: BlockId,
    transactions: Vec<BroadcastedTransaction>,
    simulation_flags: Vec<SimulationFlag>,
) -> RpcResult<Vec<SimulatedTransaction>>
where
    A: ChainApi<Block = B> + 'static,
    B: BlockT,
    BE: Backend<B> + 'static,
    G: GenesisProvider + Send + Sync + 'static,
    C: HeaderBackend<B> + BlockBackend<B> + StorageProvider<B, BE> + 'static,
    C: ProvideRuntimeApi<B>,
    C::Api: StarknetRuntimeApi<B> + ConvertTransactionRuntimeApi<B>,
    P: TransactionPool<Block = B> + 'static,
    H: HasherT + Send + Sync + 'static,
{
    let substrate_block_hash =
        starknet.substrate_block_hash_from_starknet_block(block_id).map_err(|e| StarknetRpcApiError::BlockNotFound)?;
    let chain_id = Felt252Wrapper(starknet.chain_id()?.0);
    let best_block_hash = starknet.client.info().best_hash;

    let tx_type_and_tx_iterator = transactions.into_iter().map(|tx| match tx {
        BroadcastedTransaction::Invoke(invoke_tx) => invoke_tx.try_into().map(|tx| (TxType::Invoke, tx)),
        BroadcastedTransaction::Declare(declare_tx) => declare_tx.try_into().map(|tx| (TxType::Declare, tx)),
        BroadcastedTransaction::DeployAccount(deploy_account_tx) => {
            deploy_account_tx.try_into().map(|tx| (TxType::DeployAccount, tx))
        }
    });
    let (tx_types, user_transactions) =
        itertools::process_results(tx_type_and_tx_iterator, |iter| iter.unzip::<_, _, Vec<_>, Vec<_>>()).map_err(
            |e| {
                error!("Failed to convert BroadcastedTransaction to UserTransaction: {e}");
                StarknetRpcApiError::InternalServerError
            },
        )?;

    let simulation_flags = SimulationFlags::from(simulation_flags);

    let res = starknet
        .client
        .runtime_api()
        .simulate_transactions(substrate_block_hash, user_transactions, simulation_flags)
        .map_err(|e| {
            error!("Request parameters error: {e}");
            StarknetRpcApiError::InternalServerError
        })?
        .map_err(|e| {
            error!("Failed to call function: {:#?}", e);
            StarknetRpcApiError::ContractError
        })?;

    let storage_override = starknet.overrides.for_block_hash(starknet.client.as_ref(), substrate_block_hash);
    let simulated_transactions =
        tx_execution_infos_to_simulated_transactions(&**storage_override, substrate_block_hash, tx_types, res)
            .map_err(StarknetRpcApiError::from)?;

    Ok(simulated_transactions)
}

fn tx_execution_infos_to_simulated_transactions<B: BlockT>(
    storage_override: &dyn StorageOverride<B>,
    substrate_block_hash: B::Hash,
    tx_types: Vec<TxType>,
    transaction_execution_results: Vec<
        Result<TransactionExecutionInfo, PlaceHolderErrorTypeForFailedStarknetExecution>,
    >,
) -> Result<Vec<SimulatedTransaction>, ConvertCallInfoToExecuteInvocationError> {
    let mut results = vec![];
    for (tx_type, res) in tx_types.into_iter().zip(transaction_execution_results.into_iter()) {
        match res {
            Ok(tx_exec_info) => {
                let transaction_trace =
                    tx_execution_infos_to_tx_trace(storage_override, substrate_block_hash, tx_type, &tx_exec_info)?;
                let gas_consumed =
                    tx_exec_info.execute_call_info.as_ref().map(|x| x.execution.gas_consumed).unwrap_or_default();
                let overall_fee = tx_exec_info.actual_fee.0 as u64;
                // TODO: Shouldn't the gas price be taken from the block header instead?
                let gas_price = if gas_consumed > 0 { overall_fee / gas_consumed } else { 0 };

                results.push(SimulatedTransaction {
                    transaction_trace,
                    fee_estimation: FeeEstimate { gas_consumed, gas_price, overall_fee },
                });
            }
            Err(_) => {
                return Err(ConvertCallInfoToExecuteInvocationError::TransactionExecutionFailed);
            }
        }
    }

    Ok(results)
}

// fn try_get_function_invocation_from_call_info(
//     call_info: &CallInfo,
// ) -> Result<starknet_core::types::FunctionInvocation, TryFuntionInvocationFromCallInfoError> {
//   let messages = collect_call_info_ordered_messages(call_info); let events =
//   blockifier_to_starknet_rs_ordered_events(&call_info.execution.events);

//     let inner_calls =
//         call_info.inner_calls.iter().map(try_get_function_invocation_from_call_info).
// collect::<Result<_, _>>()?;

//     call_info.get_sorted_l2_to_l1_payloads_length()?;

//     let entry_point_type = match call_info.call.entry_point_type {
//         starknet_api::deprecated_contract_class::EntryPointType::Constructor => {
//             starknet_core::types::EntryPointType::Constructor
//         }
//         starknet_api::deprecated_contract_class::EntryPointType::External => {
//             starknet_core::types::EntryPointType::External
//         }
//         starknet_api::deprecated_contract_class::EntryPointType::L1Handler => {
//             starknet_core::types::EntryPointType::L1Handler
//         }
//     };

//     let call_type = match call_info.call.call_type {
//         blockifier::execution::entry_point::CallType::Call =>
// starknet_core::types::CallType::Call,
//         blockifier::execution::entry_point::CallType::Delegate =>
// starknet_core::types::CallType::Delegate,     };

//     // The class hash in the call_info is computed during execution and will be set here.
//     let class_hash =
// FieldElement::from(Felt252Wrapper::from(call_info.call.class_hash.expect("Class hash should be
// computed after execution").0));

//     Ok(starknet_core::types::FunctionInvocation {
//         contract_address:
// FieldElement::from(Felt252Wrapper::from(call_info.call.storage_address.0.0)),
//         entry_point_selector:
// FieldElement::from(Felt252Wrapper::from(call_info.call.entry_point_selector.0)),
//         calldata: call_info.call.calldata.0.iter()
//             .map(|x| FieldElement::from(Felt252Wrapper::from(*x)))
//             .collect(),
//         caller_address:
// FieldElement::from(Felt252Wrapper::from(call_info.call.caller_address.0.0)),         class_hash,
//         entry_point_type,
//         call_type,
//         result: call_info.execution.retdata.0.iter()
//             .map(|x| FieldElement::from(Felt252Wrapper::from(*x)))
//             .collect(),
//         calls: inner_calls,
//         events,
//         messages,
//     })

// }

// #[derive(Error, Debug)]
// pub enum ConvertCallInfoToExecuteInvocationError {
//     #[error("One of the simulated transaction failed")]
//     TransactionExecutionFailed,
//     #[error(transparent)]
//     GetFunctionInvocation(#[from] TryFuntionInvocationFromCallInfoError),
// }

// impl From<ConvertCallInfoToExecuteInvocationError> for StarknetRpcApiError {
//     fn from(err: ConvertCallInfoToExecuteInvocationError) -> Self {
//         match err {
//             ConvertCallInfoToExecuteInvocationError::TransactionExecutionFailed =>
// StarknetRpcApiError::ContractError,
// ConvertCallInfoToExecuteInvocationError::GetFunctionInvocation(_) => {
// StarknetRpcApiError::InternalServerError             }
//         }
//     }
// }
