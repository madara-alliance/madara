use std::collections::HashMap;

use blockifier::execution::call_info::CallInfo;
use blockifier::state::cached_state::CommitmentStateDiff;
use dp_convert::ToFelt;
use dp_transactions::TxType;
use starknet_api::core::ContractAddress;
use starknet_core::types::{
    ComputationResources, DataAvailabilityResources, DataResources, DeclareTransactionTrace,
    DeployAccountTransactionTrace, ExecuteInvocation, ExecutionResources, Felt, InvokeTransactionTrace,
    L1HandlerTransactionTrace, NonceUpdate, RevertedInvocation, StateDiff, TransactionTrace,
};

use super::lib::*;
use crate::{utils::execution::ExecutionResult, Starknet};

pub fn collect_call_info_ordered_messages(call_info: &CallInfo) -> Vec<starknet_core::types::OrderedMessage> {
    call_info
        .execution
        .l2_to_l1_messages
        .iter()
        .enumerate()
        .map(|(index, message)| starknet_core::types::OrderedMessage {
            order: index as u64,
            payload: message.message.payload.0.iter().map(ToFelt::to_felt).collect(),
            to_address: message.message.to_address.0.to_felt(),
            from_address: call_info.call.storage_address.to_felt(),
        })
        .collect()
}

fn blockifier_to_starknet_rs_ordered_events(
    ordered_events: &[blockifier::execution::call_info::OrderedEvent],
) -> Vec<starknet_core::types::OrderedEvent> {
    ordered_events
        .iter()
        .map(|event| starknet_core::types::OrderedEvent {
            order: event.order as u64,
            keys: event.event.keys.iter().map(ToFelt::to_felt).collect(),
            data: event.event.data.0.iter().map(ToFelt::to_felt).collect(),
        })
        .collect()
}

fn try_get_funtion_invocation_from_call_info(
    starknet: &Starknet,
    call_info: &CallInfo,
    class_hash_cache: &mut HashMap<ContractAddress, Felt>,
    block_number: u64,
) -> Result<starknet_core::types::FunctionInvocation, TryFuntionInvocationFromCallInfoError> {
    let messages = collect_call_info_ordered_messages(call_info);
    let events = blockifier_to_starknet_rs_ordered_events(&call_info.execution.events);

    let inner_calls = call_info
        .inner_calls
        .iter()
        .map(|call| try_get_funtion_invocation_from_call_info(starknet, call, class_hash_cache, block_number))
        .collect::<Result<_, _>>()?;

    // TODO: check why this is here
    // call_info.get_sorted_l2_to_l1_payloads_length()?;

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
        class_hash.to_felt()
    } else if let Some(cached_hash) = class_hash_cache.get(&call_info.call.storage_address) {
        *cached_hash
    } else {
        // Compute and cache the class hash
        let Ok(Some(class_hash)) =
            starknet.backend.contract_class_hash().get_at(&call_info.call.storage_address.to_felt(), block_number)
        else {
            return Err(TryFuntionInvocationFromCallInfoError::ContractNotFound);
        };

        class_hash_cache.insert(call_info.call.storage_address, class_hash);

        class_hash
    };

    let computation_resources = computation_resources(&call_info.resources);

    Ok(starknet_core::types::FunctionInvocation {
        contract_address: call_info.call.storage_address.0.to_felt(),
        entry_point_selector: call_info.call.entry_point_selector.0.to_felt(),
        calldata: call_info.call.calldata.0.iter().map(|x| x.to_felt()).collect(),
        caller_address: call_info.call.caller_address.0.to_felt(),
        class_hash,
        entry_point_type,
        call_type,
        result: call_info.execution.retdata.0.iter().map(|x| x.to_felt()).collect(),
        calls: inner_calls,
        events,
        messages,
        execution_resources: computation_resources,
    })
}

pub fn tx_execution_infos_to_tx_trace(
    starknet: &Starknet,
    executions_result: &ExecutionResult,
    block_number: u64,
) -> Result<TransactionTrace, ConvertCallInfoToExecuteInvocationError> {
    let mut class_hash_cache: HashMap<ContractAddress, Felt> = HashMap::new();

    let ExecutionResult { tx_type, execution_info, state_diff, .. } = executions_result;

    let computation_resources = resources_mapping(&execution_info.actual_resources.0, 0, 0);

    let execution_resources = ExecutionResources {
        computation_resources,
        data_resources: DataResources {
            data_availability: DataAvailabilityResources {
                l1_gas: execution_info.da_gas.l1_gas as u64,
                l1_data_gas: execution_info.da_gas.l1_data_gas as u64,
            },
        },
    };

    let state_diff = match state_diff_is_empty(&state_diff) {
        true => None,
        false => Some(to_state_diff(state_diff)),
    };

    // If simulated with `SimulationFlag::SkipValidate` this will be `None`
    // therefore we cannot unwrap it
    let validate_invocation = execution_info
        .validate_call_info
        .as_ref()
        .map(|call_info| {
            try_get_funtion_invocation_from_call_info(starknet, call_info, &mut class_hash_cache, block_number)
        })
        .transpose()?;

    let execute_function_invocation = execution_info
        .execute_call_info
        .as_ref()
        .map(|call_info| {
            try_get_funtion_invocation_from_call_info(starknet, call_info, &mut class_hash_cache, block_number)
        })
        .transpose()?;

    // If simulated with `SimulationFlag::SkipFeeCharge` this will be `None`
    // therefore we cannot unwrap it
    let fee_transfer_invocation = execution_info
        .fee_transfer_call_info
        .as_ref()
        .map(|call_info| {
            try_get_funtion_invocation_from_call_info(starknet, call_info, &mut class_hash_cache, block_number)
        })
        .transpose()?;

    let tx_trace = match tx_type {
        TxType::Invoke => TransactionTrace::Invoke(InvokeTransactionTrace {
            validate_invocation,
            execute_invocation: if let Some(e) = &execution_info.revert_error {
                ExecuteInvocation::Reverted(RevertedInvocation { revert_reason: e.clone() })
            } else {
                ExecuteInvocation::Success(
                    execute_function_invocation
                        .ok_or(ConvertCallInfoToExecuteInvocationError::MissingFunctionInvocation)?,
                )
            },
            fee_transfer_invocation,
            state_diff,
            execution_resources,
        }),
        TxType::Declare => TransactionTrace::Declare(DeclareTransactionTrace {
            validate_invocation,
            fee_transfer_invocation,
            state_diff,
            execution_resources,
        }),
        TxType::DeployAccount => {
            TransactionTrace::DeployAccount(DeployAccountTransactionTrace {
                validate_invocation,
                constructor_invocation: execute_function_invocation
                    .ok_or(ConvertCallInfoToExecuteInvocationError::MissingFunctionInvocation)?,
                fee_transfer_invocation,
                // TODO(#1291): Compute state diff correctly
                state_diff,
                execution_resources,
            })
        }
        TxType::L1Handler => TransactionTrace::L1Handler(L1HandlerTransactionTrace {
            function_invocation: execute_function_invocation
                .ok_or(ConvertCallInfoToExecuteInvocationError::MissingFunctionInvocation)?,
            state_diff,
            execution_resources,
        }),
    };

    Ok(tx_trace)
}

pub(crate) fn computation_resources(
    vm_resources: &cairo_vm::vm::runners::cairo_runner::ExecutionResources,
) -> ComputationResources {
    let steps = vm_resources.n_steps as u64;
    let memory_holes = vm_resources.n_memory_holes as u64;
    resources_mapping(&vm_resources.builtin_instance_counter, steps, memory_holes)
}

pub(crate) fn resources_mapping(
    builtin_mapping: &HashMap<String, usize>,
    steps: u64,
    memory_holes: u64,
) -> ComputationResources {
    let memory_holes = match memory_holes {
        0 => None,
        n => Some(n),
    };

    let range_check_builtin_applications = builtin_mapping.get("range_check_builtin").map(|&value| value as u64);
    let pedersen_builtin_applications = builtin_mapping.get("pedersen_builtin").map(|&value| value as u64);
    let poseidon_builtin_applications = builtin_mapping.get("poseidon_builtin").map(|&value| value as u64);
    let ec_op_builtin_applications = builtin_mapping.get("ec_op_builtin").map(|&value| value as u64);
    let ecdsa_builtin_applications = builtin_mapping.get("ecdsa_builtin").map(|&value| value as u64);
    let bitwise_builtin_applications = builtin_mapping.get("bitwise_builtin").map(|&value| value as u64);
    let keccak_builtin_applications = builtin_mapping.get("keccak_builtin").map(|&value| value as u64);
    let segment_arena_builtin = builtin_mapping.get("segment_arena_builtin").map(|&value| value as u64);

    ComputationResources {
        steps,
        memory_holes,
        range_check_builtin_applications,
        pedersen_builtin_applications,
        poseidon_builtin_applications,
        ec_op_builtin_applications,
        ecdsa_builtin_applications,
        bitwise_builtin_applications,
        keccak_builtin_applications,
        segment_arena_builtin,
    }
}

pub(crate) fn to_state_diff(commitment_state_diff: &CommitmentStateDiff) -> StateDiff {
    StateDiff {
        storage_diffs: commitment_state_diff
            .storage_updates
            .iter()
            .map(|(address, updates)| {
                let storage_entries = updates
                    .into_iter()
                    .map(|(key, value)| starknet_core::types::StorageEntry {
                        key: key.to_felt(),
                        value: value.to_felt(),
                    })
                    .collect();
                starknet_core::types::ContractStorageDiffItem { address: address.to_felt(), storage_entries }
            })
            .collect(),
        deprecated_declared_classes: vec![],
        declared_classes: vec![],
        deployed_contracts: vec![],
        replaced_classes: vec![],
        nonces: commitment_state_diff
            .address_to_nonce
            .iter()
            .map(|(address, nonce)| NonceUpdate { contract_address: address.to_felt(), nonce: nonce.to_felt() })
            .collect(),
    }
}

pub(crate) fn state_diff_is_empty(commitment_state_diff: &CommitmentStateDiff) -> bool {
    commitment_state_diff.address_to_class_hash.is_empty()
        && commitment_state_diff.address_to_nonce.is_empty()
        && commitment_state_diff.storage_updates.is_empty()
        && commitment_state_diff.class_hash_to_compiled_class_hash.is_empty()
}
