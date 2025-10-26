use crate::{ExecutionResult, TransactionExecutionError};
use blockifier::state::cached_state::CommitmentStateDiff;
use blockifier::{blockifier_versioned_constants::VersionedConstants, execution::call_info::CallInfo};
use cairo_vm::types::builtin_name::BuiltinName;
use mp_convert::{Felt, ToFelt};
use mp_rpc::v0_9_0::ExecutionResources;
use mp_rpc::{
    v0_7_1::{CallType, ComputationResources, EntryPointType, FunctionCall, MsgToL1, OrderedEvent, OrderedMessage},
    v0_8_1::InnerCallExecutionResources,
};
use starknet_api::executable_transaction::TransactionType;
use starknet_api::transaction::fields::GasVectorComputationMode;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct FunctionInvocation {
    pub calldata: Arc<Vec<Felt>>,
    pub contract_address: Felt,
    pub selector: Felt,
    pub call_type: CallType,
    pub caller_address: Felt,
    pub internal_calls: Vec<FunctionInvocation>,
    pub class_hash: Felt,
    pub entry_point_type: EntryPointType,
    pub events: Vec<OrderedEvent>,
    pub messages: Vec<OrderedMessage>,
    pub result: Vec<Felt>,
    pub execution_resources: InnerCallExecutionResources,
    pub computation_resources: ComputationResources,
    pub is_reverted: bool,
}

#[derive(thiserror::Error, Debug)]
pub enum ConvertCallInfoToExecuteInvocationError {
    #[error(transparent)]
    GetFunctionInvocation(#[from] TryFuntionInvocationFromCallInfoError),
    #[error("Missing FunctionInvocation")]
    MissingFunctionInvocation,
}

#[derive(thiserror::Error, Debug)]
pub enum TryFuntionInvocationFromCallInfoError {
    #[error(transparent)]
    TransactionExecution(#[from] TransactionExecutionError),
}

impl FunctionInvocation {
    pub fn from_call_info(
        call_info: &CallInfo,
        versioned_constants: &VersionedConstants,
        gas_vector_computation_mode: &GasVectorComputationMode,
    ) -> Self {
        let gas_consumed = call_info
            .summarize(versioned_constants)
            .to_partial_gas_vector(versioned_constants, gas_vector_computation_mode);

        let messages = collect_call_info_ordered_messages(call_info);
        let events = collect_call_info_ordered_events(&call_info.execution.events);

        let internal_calls = call_info
            .inner_calls
            .iter()
            .map(|call_info| Self::from_call_info(call_info, versioned_constants, gas_vector_computation_mode))
            .collect();

        Self {
            calldata: call_info.call.calldata.0.clone(),
            contract_address: call_info.call.storage_address.to_felt(),
            selector: call_info.call.entry_point_selector.to_felt(),
            call_type: match call_info.call.call_type {
                blockifier::execution::entry_point::CallType::Call => CallType::Regular,
                blockifier::execution::entry_point::CallType::Delegate => CallType::Delegate,
            },
            caller_address: call_info.call.caller_address.to_felt(),
            internal_calls,
            class_hash: call_info.call.class_hash.unwrap_or_default().to_felt(),
            entry_point_type: match call_info.call.entry_point_type {
                starknet_api::contract_class::EntryPointType::Constructor => EntryPointType::Constructor,
                starknet_api::contract_class::EntryPointType::External => EntryPointType::External,
                starknet_api::contract_class::EntryPointType::L1Handler => EntryPointType::L1Handler,
            },
            events,
            messages,
            result: call_info.execution.retdata.0.clone(),
            execution_resources: InnerCallExecutionResources {
                l1_gas: gas_consumed.l1_gas.0.into(),
                l2_gas: gas_consumed.l2_gas.0.into(),
            },
            is_reverted: call_info.execution.failed,
            computation_resources: computation_resources_v0_7(&call_info.resources),
        }
    }

    pub fn into_rpc_v0_7(self) -> mp_rpc::v0_7_1::FunctionInvocation {
        mp_rpc::v0_7_1::FunctionInvocation {
            function_call: FunctionCall {
                calldata: self.calldata,
                contract_address: self.contract_address,
                entry_point_selector: self.selector,
            },
            caller_address: self.caller_address,
            class_hash: self.class_hash,
            entry_point_type: self.entry_point_type,
            call_type: self.call_type,
            result: self.result,
            calls: self.internal_calls.into_iter().map(|c| c.into_rpc_v0_7()).collect(),
            events: self.events,
            messages: self.messages,
            execution_resources: self.computation_resources,
        }
    }
    pub fn into_rpc_v0_8(self) -> mp_rpc::v0_8_1::FunctionInvocation {
        mp_rpc::v0_8_1::FunctionInvocation {
            function_call: FunctionCall {
                calldata: self.calldata,
                contract_address: self.contract_address,
                entry_point_selector: self.selector,
            },
            caller_address: self.caller_address,
            class_hash: self.class_hash,
            entry_point_type: self.entry_point_type,
            call_type: self.call_type,
            result: self.result,
            calls: self.internal_calls.into_iter().map(|c| c.into_rpc_v0_8()).collect(),
            events: self.events,
            messages: self.messages,
            execution_resources: self.execution_resources,
            is_reverted: self.is_reverted,
        }
    }
}

pub fn execution_result_to_tx_trace_v0_7(
    executions_result: &ExecutionResult,
    versioned_constants: &VersionedConstants,
) -> Result<mp_rpc::v0_7_1::TransactionTrace, ConvertCallInfoToExecuteInvocationError> {
    let ExecutionResult { tx_type, execution_info, state_diff, .. } = executions_result;

    let state_diff = match state_diff_is_empty(state_diff) {
        true => None,
        false => Some(to_state_diff(state_diff)),
    };

    let validate_invocation = execution_info.validate_call_info.as_ref().map(|call_info| {
        FunctionInvocation::from_call_info(
            call_info,
            versioned_constants,
            &executions_result.gas_vector_computation_mode,
        )
        .into_rpc_v0_7()
    });
    let execute_function_invocation = execution_info.execute_call_info.as_ref().map(|call_info| {
        FunctionInvocation::from_call_info(
            call_info,
            versioned_constants,
            &executions_result.gas_vector_computation_mode,
        )
        .into_rpc_v0_7()
    });
    let fee_transfer_invocation = execution_info.fee_transfer_call_info.as_ref().map(|call_info| {
        FunctionInvocation::from_call_info(
            call_info,
            versioned_constants,
            &executions_result.gas_vector_computation_mode,
        )
        .into_rpc_v0_7()
    });

    let computation_resources = agregate_execution_ressources_v0_7(
        validate_invocation.as_ref().map(|value| &value.execution_resources),
        execute_function_invocation.as_ref().map(|value| &value.execution_resources),
        fee_transfer_invocation.as_ref().map(|value| &value.execution_resources),
    );

    let execution_resources = mp_rpc::v0_7_1::ExecutionResources {
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
        data_availability: mp_rpc::v0_7_1::DataAvailability {
            l1_gas: execution_info.receipt.da_gas.l1_gas.0 as _,
            l1_data_gas: execution_info.receipt.da_gas.l1_data_gas.0 as _,
        },
    };

    let tx_trace = match tx_type {
        TransactionType::Declare => {
            mp_rpc::v0_7_1::TransactionTrace::Declare(mp_rpc::v0_7_1::DeclareTransactionTrace {
                validate_invocation,
                fee_transfer_invocation,
                state_diff,
                execution_resources,
            })
        }
        TransactionType::DeployAccount => {
            mp_rpc::v0_7_1::TransactionTrace::DeployAccount(mp_rpc::v0_7_1::DeployAccountTransactionTrace {
                validate_invocation,
                constructor_invocation: execute_function_invocation
                    .ok_or(ConvertCallInfoToExecuteInvocationError::MissingFunctionInvocation)?,
                fee_transfer_invocation,
                state_diff,
                execution_resources,
            })
        }
        TransactionType::InvokeFunction => {
            mp_rpc::v0_7_1::TransactionTrace::Invoke(mp_rpc::v0_7_1::InvokeTransactionTrace {
                validate_invocation,
                execute_invocation: if let Some(e) = &execution_info.revert_error {
                    mp_rpc::v0_7_1::ExecuteInvocation::Anon(mp_rpc::v0_7_1::RevertedInvocation {
                        revert_reason: e.to_string(),
                    })
                } else {
                    mp_rpc::v0_7_1::ExecuteInvocation::FunctionInvocation(
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
            if executions_result.execution_info.is_reverted() {
                // We have to map to a dummy object here, since reverted l1 handler txs are not representable in this version.
                return Ok(mp_rpc::v0_7_1::TransactionTrace::L1Handler(mp_rpc::v0_7_1::L1HandlerTransactionTrace {
                    function_invocation: mp_rpc::v0_7_1::FunctionInvocation {
                        function_call: FunctionCall {
                            calldata: Default::default(),
                            contract_address: Default::default(),
                            entry_point_selector: Default::default(),
                        },
                        call_type: CallType::Regular,
                        caller_address: Default::default(),
                        calls: Default::default(),
                        class_hash: Default::default(),
                        entry_point_type: EntryPointType::L1Handler,
                        events: Default::default(),
                        execution_resources: Default::default(),
                        messages: Default::default(),
                        result: Default::default(),
                    },
                    state_diff,
                    execution_resources,
                }));
            }
            mp_rpc::v0_7_1::TransactionTrace::L1Handler(mp_rpc::v0_7_1::L1HandlerTransactionTrace {
                function_invocation: execute_function_invocation
                    .ok_or(ConvertCallInfoToExecuteInvocationError::MissingFunctionInvocation)?,
                state_diff,
                execution_resources,
            })
        }
    };

    Ok(tx_trace)
}

pub fn execution_result_to_tx_trace_v0_8(
    executions_result: &ExecutionResult,
    versioned_constants: &VersionedConstants,
) -> Result<mp_rpc::v0_8_1::TransactionTrace, ConvertCallInfoToExecuteInvocationError> {
    let ExecutionResult { tx_type, execution_info, state_diff, .. } = executions_result;

    let state_diff = match state_diff_is_empty(state_diff) {
        true => None,
        false => Some(to_state_diff(state_diff)),
    };

    let validate_invocation = execution_info.validate_call_info.as_ref().map(|call_info| {
        FunctionInvocation::from_call_info(
            call_info,
            versioned_constants,
            &executions_result.gas_vector_computation_mode,
        )
        .into_rpc_v0_8()
    });
    let execute_function_invocation = execution_info.execute_call_info.as_ref().map(|call_info| {
        FunctionInvocation::from_call_info(
            call_info,
            versioned_constants,
            &executions_result.gas_vector_computation_mode,
        )
        .into_rpc_v0_8()
    });
    let fee_transfer_invocation = execution_info.fee_transfer_call_info.as_ref().map(|call_info| {
        FunctionInvocation::from_call_info(
            call_info,
            versioned_constants,
            &executions_result.gas_vector_computation_mode,
        )
        .into_rpc_v0_8()
    });

    let execution_resources = ExecutionResources {
        l1_gas: execution_info.receipt.gas.l1_gas.0.into(),
        l2_gas: execution_info.receipt.gas.l2_gas.0.into(),
        l1_data_gas: execution_info.receipt.gas.l1_data_gas.0.into(),
    };

    let tx_trace = match tx_type {
        TransactionType::Declare => {
            mp_rpc::v0_8_1::TransactionTrace::Declare(mp_rpc::v0_8_1::DeclareTransactionTrace {
                validate_invocation,
                fee_transfer_invocation,
                state_diff,
                execution_resources,
            })
        }
        TransactionType::DeployAccount => {
            mp_rpc::v0_8_1::TransactionTrace::DeployAccount(mp_rpc::v0_8_1::DeployAccountTransactionTrace {
                validate_invocation,
                constructor_invocation: execute_function_invocation
                    .ok_or(ConvertCallInfoToExecuteInvocationError::MissingFunctionInvocation)?,
                fee_transfer_invocation,
                state_diff,
                execution_resources,
            })
        }
        TransactionType::InvokeFunction => {
            mp_rpc::v0_8_1::TransactionTrace::Invoke(mp_rpc::v0_8_1::InvokeTransactionTrace {
                validate_invocation,
                execute_invocation: if let Some(e) = &execution_info.revert_error {
                    mp_rpc::v0_8_1::RevertibleFunctionInvocation::Anon(mp_rpc::v0_8_1::RevertedInvocation {
                        revert_reason: e.to_string(),
                    })
                } else {
                    mp_rpc::v0_8_1::RevertibleFunctionInvocation::FunctionInvocation(
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
            if executions_result.execution_info.is_reverted() {
                // We have to map to a dummy object here, since reverted l1 handler txs are not representable in this version.
                return Ok(mp_rpc::v0_8_1::TransactionTrace::L1Handler(mp_rpc::v0_8_1::L1HandlerTransactionTrace {
                    function_invocation: mp_rpc::v0_8_1::FunctionInvocation {
                        function_call: FunctionCall {
                            calldata: Default::default(),
                            contract_address: Default::default(),
                            entry_point_selector: Default::default(),
                        },
                        call_type: CallType::Regular,
                        caller_address: Default::default(),
                        calls: Default::default(),
                        class_hash: Default::default(),
                        entry_point_type: EntryPointType::L1Handler,
                        events: Default::default(),
                        execution_resources: Default::default(),
                        messages: Default::default(),
                        result: Default::default(),
                        is_reverted: true,
                    },
                    state_diff,
                    execution_resources,
                }));
            }
            mp_rpc::v0_8_1::TransactionTrace::L1Handler(mp_rpc::v0_8_1::L1HandlerTransactionTrace {
                function_invocation: execute_function_invocation
                    .ok_or(ConvertCallInfoToExecuteInvocationError::MissingFunctionInvocation)?,
                state_diff,
                execution_resources,
            })
        }
    };

    Ok(tx_trace)
}

pub fn execution_result_to_tx_trace_v0_9(
    executions_result: &ExecutionResult,
    versioned_constants: &VersionedConstants,
) -> Result<mp_rpc::v0_9_0::TransactionTrace, ConvertCallInfoToExecuteInvocationError> {
    let ExecutionResult { tx_type, execution_info, state_diff, .. } = executions_result;

    let state_diff = match state_diff_is_empty(state_diff) {
        true => None,
        false => Some(to_state_diff(state_diff)),
    };

    let validate_invocation = execution_info.validate_call_info.as_ref().map(|call_info| {
        FunctionInvocation::from_call_info(
            call_info,
            versioned_constants,
            &executions_result.gas_vector_computation_mode,
        )
        .into_rpc_v0_8()
    });
    let execute_function_invocation = execution_info.execute_call_info.as_ref().map(|call_info| {
        FunctionInvocation::from_call_info(
            call_info,
            versioned_constants,
            &executions_result.gas_vector_computation_mode,
        )
        .into_rpc_v0_8()
    });
    let fee_transfer_invocation = execution_info.fee_transfer_call_info.as_ref().map(|call_info| {
        FunctionInvocation::from_call_info(
            call_info,
            versioned_constants,
            &executions_result.gas_vector_computation_mode,
        )
        .into_rpc_v0_8()
    });

    let execution_resources = ExecutionResources {
        l1_gas: execution_info.receipt.gas.l1_gas.0.into(),
        l2_gas: execution_info.receipt.gas.l2_gas.0.into(),
        l1_data_gas: execution_info.receipt.gas.l1_data_gas.0.into(),
    };

    let tx_trace = match tx_type {
        TransactionType::Declare => {
            mp_rpc::v0_9_0::TransactionTrace::Declare(mp_rpc::v0_9_0::DeclareTransactionTrace {
                validate_invocation,
                fee_transfer_invocation,
                state_diff,
                execution_resources,
            })
        }
        TransactionType::DeployAccount => {
            mp_rpc::v0_9_0::TransactionTrace::DeployAccount(mp_rpc::v0_9_0::DeployAccountTransactionTrace {
                validate_invocation,
                constructor_invocation: execute_function_invocation
                    .ok_or(ConvertCallInfoToExecuteInvocationError::MissingFunctionInvocation)?,
                fee_transfer_invocation,
                state_diff,
                execution_resources,
            })
        }
        TransactionType::InvokeFunction => {
            mp_rpc::v0_9_0::TransactionTrace::Invoke(mp_rpc::v0_9_0::InvokeTransactionTrace {
                validate_invocation,
                execute_invocation: if let Some(e) = &execution_info.revert_error {
                    mp_rpc::v0_9_0::RevertibleFunctionInvocation::Anon(mp_rpc::v0_9_0::RevertedInvocation {
                        revert_reason: e.to_string(),
                    })
                } else {
                    mp_rpc::v0_9_0::RevertibleFunctionInvocation::FunctionInvocation(
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
            mp_rpc::v0_9_0::TransactionTrace::L1Handler(mp_rpc::v0_9_0::L1HandlerTransactionTrace {
                function_invocation: if let Some(e) = &execution_info.revert_error {
                    mp_rpc::v0_9_0::RevertibleFunctionInvocation::Anon(mp_rpc::v0_9_0::RevertedInvocation {
                        revert_reason: e.to_string(),
                    })
                } else {
                    mp_rpc::v0_9_0::RevertibleFunctionInvocation::FunctionInvocation(
                        execute_function_invocation
                            .ok_or(ConvertCallInfoToExecuteInvocationError::MissingFunctionInvocation)?,
                    )
                },
                state_diff,
                execution_resources,
            })
        }
    };

    Ok(tx_trace)
}

fn collect_call_info_ordered_messages(call_info: &CallInfo) -> Vec<mp_rpc::v0_7_1::OrderedMessage> {
    call_info
        .execution
        .l2_to_l1_messages
        .iter()
        .enumerate()
        .map(|(index, message)| mp_rpc::v0_7_1::OrderedMessage {
            order: index as u64,
            msg_to_l_1: MsgToL1 {
                payload: message.message.payload.0.to_vec(),
                to_address: message.message.to_address.to_felt(),
                from_address: call_info.call.storage_address.to_felt(),
            },
        })
        .collect()
}

fn collect_call_info_ordered_events(
    ordered_events: &[blockifier::execution::call_info::OrderedEvent],
) -> Vec<mp_rpc::v0_7_1::OrderedEvent> {
    ordered_events
        .iter()
        .map(|event| mp_rpc::v0_7_1::OrderedEvent {
            order: event.order as u64,
            event: mp_rpc::v0_7_1::EventContent {
                keys: event.event.keys.iter().map(ToFelt::to_felt).collect(),
                data: event.event.data.0.to_vec(),
            },
        })
        .collect()
}

fn computation_resources_v0_7(
    vm_resources: &cairo_vm::vm::runners::cairo_runner::ExecutionResources,
) -> mp_rpc::v0_7_1::ComputationResources {
    let steps = vm_resources.n_steps as u64;
    let memory_holes = vm_resources.n_memory_holes as u64;
    resources_mapping(&vm_resources.builtin_instance_counter, steps, memory_holes)
}

fn resources_mapping(
    builtin_mapping: &HashMap<BuiltinName, usize>,
    steps: u64,
    memory_holes: u64,
) -> mp_rpc::v0_7_1::ComputationResources {
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

    mp_rpc::v0_7_1::ComputationResources {
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

fn to_state_diff(commitment_state_diff: &CommitmentStateDiff) -> mp_rpc::v0_7_1::StateDiff {
    mp_rpc::v0_7_1::StateDiff {
        storage_diffs: commitment_state_diff
            .storage_updates
            .iter()
            .map(|(address, updates)| {
                let storage_entries = updates
                    .into_iter()
                    .map(|(key, value)| mp_rpc::v0_7_1::KeyValuePair { key: key.to_felt(), value: *value })
                    .collect();
                mp_rpc::v0_7_1::ContractStorageDiffItem { address: address.to_felt(), storage_entries }
            })
            .collect(),
        deprecated_declared_classes: vec![],
        declared_classes: vec![],
        deployed_contracts: vec![],
        replaced_classes: vec![],
        nonces: commitment_state_diff
            .address_to_nonce
            .iter()
            .map(|(address, nonce)| mp_rpc::v0_7_1::NonceUpdate {
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

fn agregate_execution_ressources_v0_7(
    a: Option<&mp_rpc::v0_7_1::ComputationResources>,
    b: Option<&mp_rpc::v0_7_1::ComputationResources>,
    c: Option<&mp_rpc::v0_7_1::ComputationResources>,
) -> mp_rpc::v0_7_1::ComputationResources {
    fn sum<'a>(
        iter: impl IntoIterator<Item = &'a mp_rpc::v0_7_1::ComputationResources>,
        getter: impl Fn(&ComputationResources) -> Option<u64>,
    ) -> Option<u64> {
        let sum = iter.into_iter().filter_map(getter).sum();
        if sum == 0 {
            None
        } else {
            Some(sum)
        }
    }
    let abc = [a, b, c].into_iter().flatten();
    mp_rpc::v0_7_1::ComputationResources {
        steps: abc.clone().map(|x| x.steps).sum(),
        memory_holes: sum(abc.clone(), |x| x.memory_holes),
        range_check_builtin_applications: sum(abc.clone(), |x| x.range_check_builtin_applications),
        pedersen_builtin_applications: sum(abc.clone(), |x| x.pedersen_builtin_applications),
        poseidon_builtin_applications: sum(abc.clone(), |x| x.poseidon_builtin_applications),
        ec_op_builtin_applications: sum(abc.clone(), |x| x.ec_op_builtin_applications),
        ecdsa_builtin_applications: sum(abc.clone(), |x| x.ecdsa_builtin_applications),
        bitwise_builtin_applications: sum(abc.clone(), |x| x.bitwise_builtin_applications),
        keccak_builtin_applications: sum(abc.clone(), |x| x.keccak_builtin_applications),
        segment_arena_builtin: sum(abc.clone(), |x| x.segment_arena_builtin),
    }
}
