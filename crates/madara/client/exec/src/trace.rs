use std::collections::HashMap;

use blockifier::state::cached_state::CommitmentStateDiff;
use blockifier::{execution::call_info::CallInfo, transaction::transaction_types::TransactionType};
use cairo_vm::types::builtin_name::BuiltinName;
use mp_convert::ToFelt;
use starknet_types_core::felt::Felt;
use starknet_types_rpc::{FunctionCall, MsgToL1};

use crate::{ExecutionResult, TransactionExecutionError};

#[derive(thiserror::Error, Debug)]
pub enum ConvertCallInfoToExecuteInvocationError {
    #[error("One of the simulated transaction failed")]
    TransactionExecutionFailed,
    #[error(transparent)]
    GetFunctionInvocation(#[from] TryFuntionInvocationFromCallInfoError),
    #[error("Missing FunctionInvocation")]
    MissingFunctionInvocation,
}

#[derive(thiserror::Error, Debug)]
pub enum TryFuntionInvocationFromCallInfoError {
    #[error(transparent)]
    TransactionExecution(#[from] TransactionExecutionError),
    #[error("No contract found at the Call contract_address")]
    ContractNotFound,
}

pub fn execution_result_to_tx_trace(
    executions_result: &ExecutionResult,
) -> Result<starknet_types_rpc::TransactionTrace<Felt>, ConvertCallInfoToExecuteInvocationError> {
    let ExecutionResult { tx_type, execution_info, state_diff, .. } = executions_result;

    let state_diff = match state_diff_is_empty(state_diff) {
        true => None,
        false => Some(to_state_diff(state_diff)),
    };

    let validate_invocation =
        execution_info.validate_call_info.as_ref().map(try_get_funtion_invocation_from_call_info).transpose()?;
    let execute_function_invocation =
        execution_info.execute_call_info.as_ref().map(try_get_funtion_invocation_from_call_info).transpose()?;
    let fee_transfer_invocation =
        execution_info.fee_transfer_call_info.as_ref().map(try_get_funtion_invocation_from_call_info).transpose()?;

    let computation_resources = aggregate_execution_ressources(
        validate_invocation.as_ref().map(|value| value.execution_resources.clone()).as_ref(),
        execute_function_invocation.as_ref().map(|value| value.execution_resources.clone()).as_ref(),
        fee_transfer_invocation.as_ref().map(|value| value.execution_resources.clone()).as_ref(),
    );

    let execution_resources = starknet_types_rpc::ExecutionResources {
        bitwise_builtin_applications: computation_resources.bitwise_builtin_applications,
        ec_op_builtin_applications: computation_resources.ec_op_builtin_applications,
        ecdsa_builtin_applications: computation_resources.ecdsa_builtin_applications,
        keccak_builtin_applications: computation_resources.keccak_builtin_applications,
        memory_holes: computation_resources.memory_holes,
        pedersen_builtin_applications: computation_resources.pedersen_builtin_applications,
        poseidon_builtin_applications: computation_resources.poseidon_builtin_applications,
        range_check_builtin_applications: computation_resources.range_check_builtin_applications,
        segment_arena_builtin: computation_resources.segment_arena_builtin,
        steps: computation_resources.steps,
        data_availability: starknet_types_rpc::DataAvailability {
            l1_gas: execution_info.transaction_receipt.da_gas.l1_gas,
            l1_data_gas: execution_info.transaction_receipt.da_gas.l1_data_gas,
        },
    };

    let tx_trace = match tx_type {
        TransactionType::Declare => {
            starknet_types_rpc::TransactionTrace::Declare(starknet_types_rpc::DeclareTransactionTrace {
                validate_invocation,
                fee_transfer_invocation,
                state_diff,
                execution_resources,
            })
        }
        TransactionType::DeployAccount => {
            starknet_types_rpc::TransactionTrace::DeployAccount(starknet_types_rpc::DeployAccountTransactionTrace {
                validate_invocation,
                constructor_invocation: execute_function_invocation
                    .ok_or(ConvertCallInfoToExecuteInvocationError::MissingFunctionInvocation)?,
                fee_transfer_invocation,
                state_diff,
                execution_resources,
            })
        }
        TransactionType::InvokeFunction => {
            starknet_types_rpc::TransactionTrace::Invoke(starknet_types_rpc::InvokeTransactionTrace {
                validate_invocation,
                execute_invocation: if let Some(e) = &execution_info.revert_error {
                    starknet_types_rpc::ExecuteInvocation::Anon(starknet_types_rpc::RevertedInvocation {
                        revert_reason: e.clone(),
                    })
                } else {
                    starknet_types_rpc::ExecuteInvocation::FunctionInvocation(
                        execute_function_invocation
                            .ok_or(ConvertCallInfoToExecuteInvocationError::MissingFunctionInvocation)?,
                    )
                },
                fee_transfer_invocation,
                state_diff,
                execution_resources,
            })
        }
        TransactionType::L1Handler => {
            starknet_types_rpc::TransactionTrace::L1Handler(starknet_types_rpc::L1HandlerTransactionTrace {
                function_invocation: execute_function_invocation
                    .ok_or(ConvertCallInfoToExecuteInvocationError::MissingFunctionInvocation)?,
                state_diff,
                execution_resources,
            })
        }
    };

    Ok(tx_trace)
}

fn try_get_funtion_invocation_from_call_info(
    call_info: &CallInfo,
) -> Result<starknet_types_rpc::FunctionInvocation<Felt>, TryFuntionInvocationFromCallInfoError> {
    let messages = collect_call_info_ordered_messages(call_info);
    let events = collect_call_info_ordered_events(&call_info.execution.events);

    let inner_calls =
        call_info.inner_calls.iter().map(try_get_funtion_invocation_from_call_info).collect::<Result<_, _>>()?;

    let entry_point_type = match call_info.call.entry_point_type {
        starknet_api::deprecated_contract_class::EntryPointType::Constructor => {
            starknet_types_rpc::EntryPointType::Constructor
        }
        starknet_api::deprecated_contract_class::EntryPointType::External => {
            starknet_types_rpc::EntryPointType::External
        }
        starknet_api::deprecated_contract_class::EntryPointType::L1Handler => {
            starknet_types_rpc::EntryPointType::L1Handler
        }
    };

    let call_type = match call_info.call.call_type {
        blockifier::execution::entry_point::CallType::Call => starknet_types_rpc::CallType::Regular,
        blockifier::execution::entry_point::CallType::Delegate => starknet_types_rpc::CallType::Delegate,
    };

    // Field `class_hash` into `FunctionInvocation` should be an Option
    let class_hash = call_info.call.class_hash.map(ToFelt::to_felt).unwrap_or_default();
    let computation_resources = computation_resources(&call_info.resources);

    Ok(starknet_types_rpc::FunctionInvocation {
        function_call: FunctionCall {
            contract_address: call_info.call.storage_address.0.to_felt(),
            entry_point_selector: call_info.call.entry_point_selector.0,
            calldata: call_info.call.calldata.0.to_vec(),
        },
        caller_address: call_info.call.caller_address.0.to_felt(),
        class_hash,
        entry_point_type,
        call_type,
        result: call_info.execution.retdata.0.to_vec(),
        calls: inner_calls,
        events,
        messages,
        execution_resources: computation_resources,
    })
}

fn collect_call_info_ordered_messages(call_info: &CallInfo) -> Vec<starknet_types_rpc::OrderedMessage<Felt>> {
    call_info
        .execution
        .l2_to_l1_messages
        .iter()
        .enumerate()
        .map(|(index, message)| starknet_types_rpc::OrderedMessage {
            order: index as u64,
            msg_to_l_1: MsgToL1 {
                payload: message.message.payload.0.to_vec(),
                to_address: message.message.to_address.0.to_felt(),
                from_address: call_info.call.storage_address.to_felt(),
            },
        })
        .collect()
}

fn collect_call_info_ordered_events(
    ordered_events: &[blockifier::execution::call_info::OrderedEvent],
) -> Vec<starknet_types_rpc::OrderedEvent<Felt>> {
    ordered_events
        .iter()
        .map(|event| starknet_types_rpc::OrderedEvent {
            order: event.order as u64,
            event: starknet_types_rpc::EventContent {
                keys: event.event.keys.iter().map(ToFelt::to_felt).collect(),
                data: event.event.data.0.to_vec(),
            },
        })
        .collect()
}

fn computation_resources(
    vm_resources: &cairo_vm::vm::runners::cairo_runner::ExecutionResources,
) -> starknet_types_rpc::ComputationResources {
    let steps = vm_resources.n_steps as u64;
    let memory_holes = vm_resources.n_memory_holes as u64;
    resources_mapping(&vm_resources.builtin_instance_counter, steps, memory_holes)
}

fn resources_mapping(
    builtin_mapping: &HashMap<BuiltinName, usize>,
    steps: u64,
    memory_holes: u64,
) -> starknet_types_rpc::ComputationResources {
    let memory_holes = match memory_holes {
        0 => None,
        n => Some(n),
    };

    let range_check_builtin_applications = builtin_mapping.get(&BuiltinName::range_check).map(|&value| value as u64);
    let pedersen_builtin_applications = builtin_mapping.get(&BuiltinName::pedersen).map(|&value| value as u64);
    let poseidon_builtin_applications = builtin_mapping.get(&BuiltinName::poseidon).map(|&value| value as u64);
    let ec_op_builtin_applications = builtin_mapping.get(&BuiltinName::ec_op).map(|&value| value as u64);
    let ecdsa_builtin_applications = builtin_mapping.get(&BuiltinName::ecdsa).map(|&value| value as u64);
    let bitwise_builtin_applications = builtin_mapping.get(&BuiltinName::bitwise).map(|&value| value as u64);
    let keccak_builtin_applications = builtin_mapping.get(&BuiltinName::keccak).map(|&value| value as u64);
    let segment_arena_builtin = builtin_mapping.get(&BuiltinName::segment_arena).map(|&value| value as u64);

    starknet_types_rpc::ComputationResources {
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

fn to_state_diff(commitment_state_diff: &CommitmentStateDiff) -> starknet_types_rpc::StateDiff<Felt> {
    starknet_types_rpc::StateDiff {
        storage_diffs: commitment_state_diff
            .storage_updates
            .iter()
            .map(|(address, updates)| {
                let storage_entries = updates
                    .into_iter()
                    .map(|(key, value)| starknet_types_rpc::KeyValuePair { key: key.to_felt(), value: *value })
                    .collect();
                starknet_types_rpc::ContractStorageDiffItem { address: address.to_felt(), storage_entries }
            })
            .collect(),
        deprecated_declared_classes: vec![],
        declared_classes: vec![],
        deployed_contracts: vec![],
        replaced_classes: vec![],
        nonces: commitment_state_diff
            .address_to_nonce
            .iter()
            .map(|(address, nonce)| starknet_types_rpc::NonceUpdate {
                contract_address: address.to_felt(),
                nonce: nonce.to_felt(),
            })
            .collect(),
    }
}

fn state_diff_is_empty(commitment_state_diff: &CommitmentStateDiff) -> bool {
    commitment_state_diff.address_to_class_hash.is_empty()
        && commitment_state_diff.address_to_nonce.is_empty()
        && commitment_state_diff.storage_updates.is_empty()
        && commitment_state_diff.class_hash_to_compiled_class_hash.is_empty()
}

fn aggregate_execution_ressources(
    a: Option<&starknet_types_rpc::ComputationResources>,
    b: Option<&starknet_types_rpc::ComputationResources>,
    c: Option<&starknet_types_rpc::ComputationResources>,
) -> starknet_types_rpc::ComputationResources {
    starknet_types_rpc::ComputationResources {
        steps: a.map_or(0, |x| x.steps) + b.map_or(0, |x| x.steps) + c.map_or(0, |x| x.steps),
        memory_holes: {
            let sum = a.and_then(|x| x.memory_holes).unwrap_or_default()
                + b.and_then(|x| x.memory_holes).unwrap_or_default()
                + c.and_then(|x| x.memory_holes).unwrap_or_default();
            match sum {
                0 => None,
                n => Some(n),
            }
        },
        range_check_builtin_applications: {
            let sum = a.and_then(|x| x.range_check_builtin_applications).unwrap_or_default()
                + b.and_then(|x| x.range_check_builtin_applications).unwrap_or_default()
                + c.and_then(|x| x.range_check_builtin_applications).unwrap_or_default();
            match sum {
                0 => None,
                n => Some(n),
            }
        },
        pedersen_builtin_applications: {
            let sum = a.and_then(|x| x.pedersen_builtin_applications).unwrap_or_default()
                + b.and_then(|x| x.pedersen_builtin_applications).unwrap_or_default()
                + c.and_then(|x| x.pedersen_builtin_applications).unwrap_or_default();
            match sum {
                0 => None,
                n => Some(n),
            }
        },
        poseidon_builtin_applications: {
            let sum = a.and_then(|x| x.poseidon_builtin_applications).unwrap_or_default()
                + b.and_then(|x| x.poseidon_builtin_applications).unwrap_or_default()
                + c.and_then(|x| x.poseidon_builtin_applications).unwrap_or_default();
            match sum {
                0 => None,
                n => Some(n),
            }
        },
        ec_op_builtin_applications: {
            let sum = a.and_then(|x| x.ec_op_builtin_applications).unwrap_or_default()
                + b.and_then(|x| x.ec_op_builtin_applications).unwrap_or_default()
                + c.and_then(|x| x.ec_op_builtin_applications).unwrap_or_default();
            match sum {
                0 => None,
                n => Some(n),
            }
        },
        ecdsa_builtin_applications: {
            let sum = a.and_then(|x| x.ecdsa_builtin_applications).unwrap_or_default()
                + b.and_then(|x| x.ecdsa_builtin_applications).unwrap_or_default()
                + c.and_then(|x| x.ecdsa_builtin_applications).unwrap_or_default();
            match sum {
                0 => None,
                n => Some(n),
            }
        },
        bitwise_builtin_applications: {
            let sum = a.and_then(|x| x.bitwise_builtin_applications).unwrap_or_default()
                + b.and_then(|x| x.bitwise_builtin_applications).unwrap_or_default()
                + c.and_then(|x| x.bitwise_builtin_applications).unwrap_or_default();
            match sum {
                0 => None,
                n => Some(n),
            }
        },
        keccak_builtin_applications: {
            let sum = a.and_then(|x| x.keccak_builtin_applications).unwrap_or_default()
                + b.and_then(|x| x.keccak_builtin_applications).unwrap_or_default()
                + c.and_then(|x| x.keccak_builtin_applications).unwrap_or_default();
            match sum {
                0 => None,
                n => Some(n),
            }
        },
        segment_arena_builtin: {
            let sum = a.and_then(|x| x.segment_arena_builtin).unwrap_or_default()
                + b.and_then(|x| x.segment_arena_builtin).unwrap_or_default()
                + c.and_then(|x| x.segment_arena_builtin).unwrap_or_default();
            match sum {
                0 => None,
                n => Some(n),
            }
        },
    }
}
