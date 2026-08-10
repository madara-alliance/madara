use crate::errors::StarknetRpcResult;
use crate::versions::user::v0_9_0::methods::trace::trace_block_transactions::prepare_tx_for_reexecution;
use crate::{Starknet, StarknetRpcApiError};
use mc_exec::MadaraBlockViewExecutionExt;
use mc_rust_exec::blockifier_integration::RustExecStateAdapter;
use mc_rust_exec::blockifier_integration::{rust_execute_transaction_with_info, RustExecutionOutcome};
use mc_rust_exec::trace::{RustCallInfo, RustCallType, RustEntryPointType};
use mp_rpc::v0_10_0::ExecutionResources;
use mp_rpc::v0_7_1::{ContractStorageDiffItem, FunctionCall, KeyValuePair, NonceUpdate, StateDiff};
use mp_rpc::v0_8_1::{
    CallType, EntryPointType, FunctionInvocation, InnerCallExecutionResources, OrderedEvent, OrderedMessage,
    RevertibleFunctionInvocation,
};
use mp_rpc::v0_9_0::{InvokeTransactionTrace, TraceTransactionResult, TransactionTrace};
use starknet_types_core::felt::Felt;

fn zero_execution_resources() -> ExecutionResources {
    ExecutionResources { l1_gas: 0, l2_gas: 0, l1_data_gas: 0 }
}

fn zero_inner_resources() -> InnerCallExecutionResources {
    InnerCallExecutionResources { l1_gas: 0, l2_gas: 0 }
}

fn execution_resources_from_receipt(
    receipt: Option<&mc_rust_exec::trace::RustTransactionReceipt>,
) -> ExecutionResources {
    match receipt {
        Some(receipt) => ExecutionResources {
            l1_gas: receipt.gas_consumed.l1_gas as u128,
            l2_gas: receipt.gas_consumed.l2_gas as u128,
            l1_data_gas: receipt.gas_consumed.l1_data_gas as u128,
        },
        None => zero_execution_resources(),
    }
}

fn build_ordered_events(events: &[mc_rust_exec::types::Event]) -> Vec<OrderedEvent> {
    events
        .iter()
        .map(|event| OrderedEvent {
            order: event.order as u64,
            event: mp_rpc::v0_7_1::EventContent { data: event.data.clone(), keys: event.keys.clone() },
        })
        .collect()
}

fn build_ordered_messages(messages: &[mc_rust_exec::types::L2ToL1Message]) -> Vec<OrderedMessage> {
    messages
        .iter()
        .enumerate()
        .map(|(idx, msg)| OrderedMessage {
            order: idx as u64,
            msg_to_l_1: mp_rpc::v0_7_1::MsgToL1 {
                from_address: Felt::ZERO,
                to_address: msg.to_address,
                payload: msg.payload.clone(),
            },
        })
        .collect()
}

fn build_state_diff(state_diff: &mc_rust_exec::types::StateDiff) -> StateDiff {
    let storage_diffs = state_diff
        .storage_updates
        .iter()
        .map(|(addr, updates)| ContractStorageDiffItem {
            address: addr.0,
            storage_entries: updates.iter().map(|(key, value)| KeyValuePair { key: key.0, value: *value }).collect(),
        })
        .collect::<Vec<_>>();

    let nonces = state_diff
        .address_to_nonce
        .iter()
        .map(|(addr, nonce)| NonceUpdate { contract_address: addr.0, nonce: nonce.0 })
        .collect::<Vec<_>>();

    let declared_classes = state_diff
        .class_hash_to_compiled_class_hash
        .iter()
        .map(|(class_hash, compiled_class_hash)| mp_rpc::v0_7_1::NewClasses {
            class_hash: *class_hash,
            compiled_class_hash: *compiled_class_hash,
        })
        .collect::<Vec<_>>();

    let replaced_classes = state_diff
        .address_to_class_hash
        .iter()
        .map(|(addr, class_hash)| mp_rpc::v0_7_1::ReplacedClass { class_hash: *class_hash, contract_address: addr.0 })
        .collect::<Vec<_>>();

    StateDiff {
        declared_classes,
        deployed_contracts: Vec::new(),
        deprecated_declared_classes: Vec::new(),
        nonces,
        replaced_classes,
        storage_diffs,
    }
}

fn map_call_type(call_type: RustCallType) -> CallType {
    match call_type {
        RustCallType::Regular => CallType::Regular,
        RustCallType::Delegate => CallType::Delegate,
    }
}

fn map_entry_point_type(entry_point_type: RustEntryPointType) -> EntryPointType {
    match entry_point_type {
        RustEntryPointType::External => EntryPointType::External,
        RustEntryPointType::Constructor => EntryPointType::Constructor,
        RustEntryPointType::L1Handler => EntryPointType::L1Handler,
    }
}

fn build_function_invocation(info: &RustCallInfo) -> FunctionInvocation {
    FunctionInvocation {
        function_call: FunctionCall {
            calldata: info.function_call.calldata.clone().into(),
            contract_address: info.function_call.contract_address,
            entry_point_selector: info.function_call.entry_point_selector,
        },
        call_type: map_call_type(info.call_type),
        caller_address: info.caller_address,
        calls: info.inner_calls.iter().map(build_function_invocation).collect(),
        class_hash: info.class_hash,
        entry_point_type: map_entry_point_type(info.entry_point_type),
        events: build_ordered_events(&info.execution.events),
        execution_resources: zero_inner_resources(),
        messages: build_ordered_messages(&info.execution.l2_to_l1_messages),
        result: info.execution.retdata.clone(),
        is_reverted: info.execution.failed,
    }
}

pub async fn trace_transaction_rust(
    starknet: &Starknet,
    transaction_hash: Felt,
) -> StarknetRpcResult<TraceTransactionResult> {
    let view = starknet.backend.view_on_latest();
    let res = view.find_transaction_by_hash(&transaction_hash)?.ok_or(StarknetRpcApiError::TxnHashNotFound)?;
    let mut exec_context = res.block.new_execution_context_at_block_start()?;

    if exec_context.protocol_version < mc_exec::EXECUTION_UNSUPPORTED_BELOW_VERSION {
        return Err(StarknetRpcApiError::unsupported_txn_version());
    }

    let state_view = res.block.state_view();
    let previous_transactions: Vec<_> = res
        .block
        .get_executed_transactions(..res.transaction_index)?
        .into_iter()
        .map(|tx| prepare_tx_for_reexecution(&state_view, tx))
        .collect::<Result<_, _>>()?;

    let transaction_to_trace = prepare_tx_for_reexecution(&state_view, res.get_transaction()?)?;

    let exec_context = mp_utils::spawn_blocking(move || {
        exec_context.execute_transactions(previous_transactions, std::iter::empty())?;
        Ok::<_, mc_exec::Error>(exec_context)
    })
    .await?;

    let rust_state = RustExecStateAdapter::new(&exec_context.state);
    rust_state.reset_stats();

    let result = rust_execute_transaction_with_info(
        &rust_state,
        &transaction_to_trace,
        &exec_context.block_context,
        transaction_hash,
    );

    let trace = match result {
        RustExecutionOutcome::Executed(data) => {
            let exec_info = data.execution_info.as_ref();
            let execute_invocation = match exec_info.and_then(|info| info.execute_call_info.as_ref()) {
                Some(call_info) => {
                    if let Some(info) = exec_info.and_then(|info| info.revert_error.as_ref()) {
                        RevertibleFunctionInvocation::Anon(mp_rpc::v0_7_1::RevertedInvocation {
                            revert_reason: info.clone(),
                        })
                    } else {
                        RevertibleFunctionInvocation::FunctionInvocation(build_function_invocation(call_info))
                    }
                }
                None => RevertibleFunctionInvocation::Anon(mp_rpc::v0_7_1::RevertedInvocation {
                    revert_reason: "Missing execute call info".to_string(),
                }),
            };

            let validate_invocation =
                exec_info.and_then(|info| info.validate_call_info.as_ref()).map(build_function_invocation);
            let fee_transfer_invocation =
                exec_info.and_then(|info| info.fee_transfer_call_info.as_ref()).map(build_function_invocation);
            let state_diff = exec_info.map(|info| build_state_diff(&info.state_diff));

            TransactionTrace::Invoke(InvokeTransactionTrace {
                execute_invocation,
                execution_resources: execution_resources_from_receipt(exec_info.map(|info| &info.receipt)),
                fee_transfer_invocation,
                state_diff,
                validate_invocation,
            })
        }
        RustExecutionOutcome::Failed(failure) => {
            let execute_invocation =
                RevertibleFunctionInvocation::Anon(mp_rpc::v0_7_1::RevertedInvocation { revert_reason: failure.error });
            TransactionTrace::Invoke(InvokeTransactionTrace {
                execute_invocation,
                execution_resources: zero_execution_resources(),
                fee_transfer_invocation: None,
                state_diff: None,
                validate_invocation: None,
            })
        }
        RustExecutionOutcome::Skipped { reason, .. } => {
            let revert_reason = format!("Rust execution skipped: {:?}", reason);
            let execute_invocation =
                RevertibleFunctionInvocation::Anon(mp_rpc::v0_7_1::RevertedInvocation { revert_reason });
            TransactionTrace::Invoke(InvokeTransactionTrace {
                execute_invocation,
                execution_resources: zero_execution_resources(),
                fee_transfer_invocation: None,
                state_diff: None,
                validate_invocation: None,
            })
        }
    };
    Ok(TraceTransactionResult { trace })
}
