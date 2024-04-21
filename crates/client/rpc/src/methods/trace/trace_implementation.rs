use std::collections::HashMap;

use blockifier::execution::entry_point::CallInfo;
use blockifier::transaction::objects::TransactionExecutionInfo;
use jsonrpsee::core::RpcResult;
use log::error;
use mc_db::DeoxysBackend;
use mc_genesis_data_provider::GenesisProvider;
use mp_block::DeoxysBlock;
use mp_felt::Felt252Wrapper;
use mp_hashers::HasherT;
use mp_transactions::compute_hash::ComputeTransactionHash;
use mp_transactions::{Transaction, TxType, UserOrL1HandlerTransaction, UserTransaction};
use mp_types::block::{DBlockT, DHashT};
use pallet_starknet_runtime_api::{ConvertTransactionRuntimeApi, StarknetRuntimeApi};
use sc_client_api::{Backend, BlockBackend, StorageProvider};
use sc_transaction_pool::ChainApi;
use sc_transaction_pool_api::TransactionPool;
use sp_api::ProvideRuntimeApi;
use sp_blockchain::HeaderBackend;
use sp_runtime::traits::Block as BlockT;
use starknet_api::api_core::{ClassHash, ContractAddress};
use starknet_core::types::{
    BlockId, DeclareTransactionTrace, DeployAccountTransactionTrace, ExecuteInvocation, InvokeTransactionTrace,
    L1HandlerTransactionTrace, RevertedInvocation, TransactionTrace, TransactionTraceWithHash,
};
use starknet_ff::FieldElement;

use super::lib::*;
use crate::errors::StarknetRpcApiError;
use crate::utils::get_block_by_block_hash;
use crate::{Starknet, StarknetReadRpcApiServer};

pub async fn trace_block_transactions<A, BE, G, C, P, H>(
    starknet: &Starknet<A, BE, G, C, P, H>,
    block_id: BlockId,
) -> RpcResult<Vec<TransactionTraceWithHash>>
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
    let substrate_block_hash = starknet.substrate_block_hash_from_starknet_block(block_id).map_err(|e| {
        error!("Block not found: '{e}'");
        StarknetRpcApiError::BlockNotFound
    })?;

    let starknet_block = get_block_by_block_hash(starknet.client.as_ref(), substrate_block_hash).map_err(|e| {
        error!("Failed to get block for block hash {substrate_block_hash}: '{e}'");
        StarknetRpcApiError::InternalServerError
    })?;
    let chain_id = Felt252Wrapper(starknet.chain_id()?.0);
    let block_number = starknet_block.header().block_number;

    let (block_transactions, empty_transactions) =
        map_transaction_to_user_transaction(starknet, starknet_block, substrate_block_hash, chain_id, None)?;

    let previous_block_substrate_hash = get_previous_block_substrate_hash(starknet, substrate_block_hash)?;

    let execution_infos = starknet
        .client
        .runtime_api()
        .re_execute_transactions(previous_block_substrate_hash, empty_transactions.clone(), block_transactions.clone())
        .map_err(|e| {
            error!("Failed to execute runtime API call: {e}");
            StarknetRpcApiError::InternalServerError
        })?
        .map_err(|e| {
            error!("Failed to reexecute the block transactions: {e:?}");
            StarknetRpcApiError::InternalServerError
        })?
        .map_err(|_| {
            error!(
                "One of the transaction failed during it's reexecution. This should not happen, as the block has \
                 already been executed successfully in the past. There is a bug somewhere."
            );
            StarknetRpcApiError::InternalServerError
        })?;

    let storage_override = starknet.overrides.for_block_hash(starknet.client.as_ref(), substrate_block_hash);

    let traces = execution_infos
        .into_iter()
        .enumerate()
        .map(|(tx_idx, tx_exec_info)| {
            tx_execution_infos_to_tx_trace(
                &**storage_override,
                substrate_block_hash,
                // Safe to unwrap coz re_execute returns exactly one ExecutionInfo for each tx
                TxType::from(block_transactions.get(tx_idx).unwrap()),
                &tx_exec_info,
            )
            .map(|trace_root| TransactionTraceWithHash {
                transaction_hash: block_transactions[tx_idx]
                    .compute_hash::<H>(chain_id, false, Some(block_number))
                    .into(),
                trace_root,
            })
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(StarknetRpcApiError::from)?;

    Ok(traces)
}

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
            error!("Failed to get transaction's substrate block hash from mapping_db: {e}");
            StarknetRpcApiError::TxnHashNotFound
        })?
        .ok_or(StarknetRpcApiError::TxnHashNotFound)?;

    let starknet_block = get_block_by_block_hash(starknet.client.as_ref(), substrate_block_hash)?;
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

    let execution_infos = starknet
        .client
        .runtime_api()
        .re_execute_transactions(previous_block_substrate_hash, txs_to_execute_before.clone(), tx_to_trace.clone())
        .map_err(|e| {
            error!("Failed to execute runtime API call: {e}");
            StarknetRpcApiError::InternalServerError
        })?
        .map_err(|e| {
            error!("Failed to reexecute the block transactions: {e:?}");
            StarknetRpcApiError::InternalServerError
        })?
        .map_err(|_| {
            error!(
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
        TxType::from(tx_to_trace.get(0).unwrap()),
        &execution_infos[0],
    )
    .unwrap();

    let tx_trace = TransactionTraceWithHash { transaction_hash, trace_root: trace };

    Ok(tx_trace)
}

pub fn collect_call_info_ordered_messages(call_info: &CallInfo) -> Vec<starknet_core::types::OrderedMessage> {
    call_info
        .execution
        .l2_to_l1_messages
        .iter()
        .enumerate()
        .map(|(index, message)| starknet_core::types::OrderedMessage {
            order: index as u64,
            payload: message
                .message
                .payload
                .0
                .iter()
                .map(|x| {
                    let felt_wrapper: Felt252Wrapper = Felt252Wrapper::from(*x);
                    FieldElement::from(felt_wrapper)
                })
                .collect(),
            to_address: FieldElement::from_byte_slice_be(message.message.to_address.0.to_fixed_bytes().as_slice())
                .unwrap(),
            from_address: {
                let felt_wrapper: Felt252Wrapper = Felt252Wrapper::from(call_info.call.storage_address.0.0);
                FieldElement::from(felt_wrapper)
            },
        })
        .collect()
}

fn blockifier_to_starknet_rs_ordered_events(
    ordered_events: &[blockifier::execution::entry_point::OrderedEvent],
) -> Vec<starknet_core::types::OrderedEvent> {
    ordered_events
        .iter()
        .map(|event| starknet_core::types::OrderedEvent {
            order: event.order as u64, // Convert usize to u64
            keys: event.event.keys.iter().map(|key| FieldElement::from_byte_slice_be(key.0.bytes()).unwrap()).collect(),
            data: event
                .event
                .data
                .0
                .iter()
                .map(|data_item| FieldElement::from_byte_slice_be(data_item.bytes()).unwrap())
                .collect(),
        })
        .collect()
}

fn try_get_funtion_invocation_from_call_info<B: BlockT>(
    substrate_block_hash: B::Hash,
    call_info: &CallInfo,
    class_hash_cache: &mut HashMap<ContractAddress, FieldElement>,
) -> Result<starknet_core::types::FunctionInvocation, TryFuntionInvocationFromCallInfoError> {
    let messages = collect_call_info_ordered_messages(call_info);
    let events = blockifier_to_starknet_rs_ordered_events(&call_info.execution.events);

    let inner_calls = call_info
        .inner_calls
        .iter()
        .map(|call| try_get_funtion_invocation_from_call_info(substrate_block_hash, call, class_hash_cache))
        .collect::<Result<_, _>>()?;

    call_info.get_sorted_l2_to_l1_payloads_length()?;

    let entry_point_type = match call_info.call.entry_point_type {
        starknet_api::deprecated_contract_class::EntryPointType::Constructor => {
            starknet_core::types::EntryPointType::Constructor
        }
        starknet_api::deprecated_contract_class::EntryPointType::External => {
            starknet_core::types::EntryPointType::External
        }
        starknet_api::deprecated_contract_class::EntryPointType::L1Handler => {
            starknet_core::types::EntryPointType::L1Handler
        }
    };

    let call_type = match call_info.call.call_type {
        blockifier::execution::entry_point::CallType::Call => starknet_core::types::CallType::Call,
        blockifier::execution::entry_point::CallType::Delegate => starknet_core::types::CallType::Delegate,
    };

    // Blockifier call info does not give use the class_hash "if it can be deducted from the storage
    // address". We have to do this decution ourselves here
    let class_hash = if let Some(class_hash) = call_info.call.class_hash {
        let felt_wrapper: Felt252Wrapper = Felt252Wrapper::from(class_hash.0);
        FieldElement::from(felt_wrapper)
    } else if let Some(cached_hash) = class_hash_cache.get(&call_info.call.storage_address) {
        *cached_hash
    } else {
        // Compute and cache the class hash
        let computed_hash = storage_override
            .contract_class_hash_by_address(substrate_block_hash, call_info.call.storage_address)
            .ok_or_else(|| TryFuntionInvocationFromCallInfoError::ContractNotFound)?;

        let computed_hash = FieldElement::from_byte_slice_be(computed_hash.0.bytes()).unwrap();
        class_hash_cache.insert(call_info.call.storage_address, computed_hash);

        computed_hash
    };

    Ok(starknet_core::types::FunctionInvocation {
        contract_address: FieldElement::from(Felt252Wrapper::from(call_info.call.storage_address.0.0)),
        entry_point_selector: FieldElement::from(Felt252Wrapper::from(call_info.call.entry_point_selector.0)),
        calldata: call_info.call.calldata.0.iter().map(|x| FieldElement::from(Felt252Wrapper::from(*x))).collect(),
        caller_address: FieldElement::from(Felt252Wrapper::from(call_info.call.caller_address.0.0)),
        class_hash,
        entry_point_type,
        call_type,
        result: call_info.execution.retdata.0.iter().map(|x| FieldElement::from(Felt252Wrapper::from(*x))).collect(),
        calls: inner_calls,
        events,
        messages,
    })
}

pub fn tx_execution_infos_to_tx_trace<B: BlockT>(
    substrate_block_hash: B::Hash,
    tx_type: TxType,
    tx_exec_info: &TransactionExecutionInfo,
) -> Result<TransactionTrace, ConvertCallInfoToExecuteInvocationError> {
    let mut class_hash_cache: HashMap<ContractAddress, FieldElement> = HashMap::new();

    // If simulated with `SimulationFlag::SkipValidate` this will be `None`
    // therefore we cannot unwrap it
    let validate_invocation = tx_exec_info
        .validate_call_info
        .as_ref()
        .map(|call_info| {
            try_get_funtion_invocation_from_call_info(substrate_block_hash, call_info, &mut class_hash_cache)
        })
        .transpose()?;
    // If simulated with `SimulationFlag::SkipFeeCharge` this will be `None`
    // therefore we cannot unwrap it
    let fee_transfer_invocation = tx_exec_info
        .fee_transfer_call_info
        .as_ref()
        .map(|call_info| {
            try_get_funtion_invocation_from_call_info(substrate_block_hash, call_info, &mut class_hash_cache)
        })
        .transpose()?;

    let tx_trace = match tx_type {
        TxType::Invoke => TransactionTrace::Invoke(InvokeTransactionTrace {
            validate_invocation,
            execute_invocation: if let Some(e) = &tx_exec_info.revert_error {
                ExecuteInvocation::Reverted(RevertedInvocation { revert_reason: e.clone() })
            } else {
                ExecuteInvocation::Success(try_get_funtion_invocation_from_call_info(
                    substrate_block_hash,
                    // Safe to unwrap because is only `None`  for `Declare` txs
                    tx_exec_info.execute_call_info.as_ref().unwrap(),
                    &mut class_hash_cache,
                )?)
            },
            fee_transfer_invocation,
            // TODO(#1291): Compute state diff correctly
            state_diff: None,
        }),
        TxType::Declare => TransactionTrace::Declare(DeclareTransactionTrace {
            validate_invocation,
            fee_transfer_invocation,
            // TODO(#1291): Compute state diff correctly
            state_diff: None,
        }),
        TxType::DeployAccount => {
            TransactionTrace::DeployAccount(DeployAccountTransactionTrace {
                validate_invocation,
                constructor_invocation: try_get_funtion_invocation_from_call_info(
                    substrate_block_hash,
                    // Safe to unwrap because is only `None` for `Declare` txs
                    tx_exec_info.execute_call_info.as_ref().unwrap(),
                    &mut class_hash_cache,
                )?,
                fee_transfer_invocation,
                // TODO(#1291): Compute state diff correctly
                state_diff: None,
            })
        }
        TxType::L1Handler => TransactionTrace::L1Handler(L1HandlerTransactionTrace {
            function_invocation: try_get_funtion_invocation_from_call_info(
                substrate_block_hash,
                // Safe to unwrap because is only `None` for `Declare` txs
                tx_exec_info.execute_call_info.as_ref().unwrap(),
                &mut class_hash_cache,
            )?,
            state_diff: None,
        }),
    };

    Ok(tx_trace)
}

fn map_transaction_to_user_transaction<A, BE, G, C, P, H>(
    starknet: &Starknet<A, BE, G, C, P, H>,
    starknet_block: DeoxysBlock,
    substrate_block_hash: DHashT,
    chain_id: Felt252Wrapper,
    target_transaction_hash: Option<Felt252Wrapper>,
) -> Result<(Vec<UserOrL1HandlerTransaction>, Vec<UserOrL1HandlerTransaction>), StarknetRpcApiError>
where
    A: ChainApi<Block = DBlockT> + 'static,
    C: HeaderBackend<DBlockT> + BlockBackend<DBlockT> + StorageProvider<DBlockT, BE> + 'static,
    H: HasherT + Send + Sync + 'static,
    BE: Backend<DBlockT> + 'static,
{
    let mut transactions = Vec::new();
    let mut transaction_to_trace = Vec::new();
    let block_number = starknet_block.header().block_number;

    for tx in starknet_block.transactions() {
        let current_tx_hash = tx.compute_hash::<H>(chain_id, false, Some(block_number));

        if Some(current_tx_hash) == target_transaction_hash {
            let converted_tx = convert_transaction(tx, starknet, substrate_block_hash, chain_id, block_number)?;
            transaction_to_trace.push(converted_tx);
            break;
        } else {
            let converted_tx = convert_transaction(tx, starknet, substrate_block_hash, chain_id, block_number)?;
            transactions.push(converted_tx);
        }
    }

    Ok((transactions, transaction_to_trace))
}

fn convert_transaction<A, BE, G, C, P, H>(
    tx: &Transaction,
    client: &Starknet<A, BE, G, C, P, H>,
    substrate_block_hash: DHashT,
    chain_id: Felt252Wrapper,
    block_number: u64,
) -> Result<UserOrL1HandlerTransaction, StarknetRpcApiError>
where
    A: ChainApi<Block = DBlockT> + 'static,
    C: HeaderBackend<DBlockT> + BlockBackend<DBlockT> + StorageProvider<DBlockT, BE> + 'static,
    H: HasherT + Send + Sync + 'static,
    BE: Backend<DBlockT> + 'static,
{
    match tx {
        Transaction::Invoke(invoke_tx) => {
            Ok(UserOrL1HandlerTransaction::User(UserTransaction::Invoke(invoke_tx.clone())))
        }
        Transaction::DeployAccount(deploy_account_tx) => {
            Ok(UserOrL1HandlerTransaction::User(UserTransaction::DeployAccount(deploy_account_tx.clone())))
        }
        Transaction::Declare(declare_tx) => {
            let class_hash = ClassHash::from(*declare_tx.class_hash());

            let contract_class = client
                .overrides
                .for_block_hash(client.client.as_ref(), substrate_block_hash)
                .contract_class_by_class_hash(substrate_block_hash, class_hash)
                .ok_or_else(|| {
                    log::error!("Failed to retrieve contract class from hash '{class_hash}'");
                    StarknetRpcApiError::InternalServerError
                })?;

            Ok(UserOrL1HandlerTransaction::User(UserTransaction::Declare(declare_tx.clone(), contract_class)))
        }
        Transaction::L1Handler(handle_l1_message_tx) => {
            let tx_hash = handle_l1_message_tx.compute_hash::<H>(chain_id, false, Some(block_number));
            let paid_fee: starknet_api::transaction::Fee =
                DeoxysBackend::l1_handler_paid_fee().get_fee_paid_for_l1_handler_tx(tx_hash.into()).map_err(|e| {
                    error!("Failed to retrieve fee paid on l1 for tx with hash `{tx_hash:?}`: {e}");
                    StarknetRpcApiError::InternalServerError
                })?;

            Ok(UserOrL1HandlerTransaction::L1Handler(handle_l1_message_tx.clone(), paid_fee))
        }
        &mp_transactions::Transaction::Deploy(_) => todo!(),
    }
}

fn get_previous_block_substrate_hash<A, BE, G, C, P, H>(
    starknet: &Starknet<A, BE, G, C, P, H>,
    substrate_block_hash: DHashT,
) -> Result<DHashT, StarknetRpcApiError>
where
    A: ChainApi<Block = DBlockT> + 'static,
    C: HeaderBackend<DBlockT> + BlockBackend<DBlockT> + StorageProvider<DBlockT, BE> + 'static,
    C: ProvideRuntimeApi<DBlockT>,
    C::Api: StarknetRuntimeApi<DBlockT> + ConvertTransactionRuntimeApi<DBlockT>,
    H: HasherT + Send + Sync + 'static,
    BE: Backend<DBlockT> + 'static,
{
    let starknet_block = get_block_by_block_hash(starknet.client.as_ref(), substrate_block_hash).map_err(|e| {
        error!("Failed to get block for block hash {substrate_block_hash}: '{e}'");
        StarknetRpcApiError::InternalServerError
    })?;
    let block_number = starknet_block.header().block_number;
    let previous_block_number = block_number - 1;
    let substrate_block_hash =
        starknet.substrate_block_hash_from_starknet_block(BlockId::Number(previous_block_number)).map_err(|e| {
            error!("Failed to retrieve previous block substrate hash: {e}");
            StarknetRpcApiError::InternalServerError
        })?;

    Ok(substrate_block_hash)
}
