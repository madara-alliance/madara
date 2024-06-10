use std::collections::HashMap;

use blockifier::execution::call_info::CallInfo;
use blockifier::transaction::objects::TransactionExecutionInfo;
use mc_db::storage_handler;
use mp_felt::{FeltWrapper};
use mp_transactions::TxType;
use starknet_api::core::ContractAddress;
use starknet_core::types::{
    ComputationResources, DataAvailabilityResources, DataResources, DeclareTransactionTrace,
    DeployAccountTransactionTrace, ExecuteInvocation, ExecutionResources, InvokeTransactionTrace,
    L1HandlerTransactionTrace, RevertedInvocation, TransactionTrace,
};
use starknet_ff::FieldElement;

use super::lib::*;

pub fn collect_call_info_ordered_messages(call_info: &CallInfo) -> Vec<starknet_core::types::OrderedMessage> {
    call_info
        .execution
        .l2_to_l1_messages
        .iter()
        .enumerate()
        .map(|(index, message)| 
            starknet_core::types::OrderedMessage {
                order: index as u64,
                payload: message
                    .message
                    .payload
                    .0
                    .iter()
                    .map(|x| x.into_field_element())
                    .collect(),
                to_address: FieldElement::from_byte_slice_be(message.message.to_address.0.to_fixed_bytes().as_slice())
                    .unwrap(),
                from_address: call_info.call.storage_address.0.0.into_field_element()
            }
        ).collect()
}

fn blockifier_to_starknet_rs_ordered_events(
    ordered_events: &[blockifier::execution::call_info::OrderedEvent],
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

fn try_get_funtion_invocation_from_call_info(
    call_info: &CallInfo,
    class_hash_cache: &mut HashMap<ContractAddress, FieldElement>,
    block_number: u64,
) -> Result<starknet_core::types::FunctionInvocation, TryFuntionInvocationFromCallInfoError> {
    let messages = collect_call_info_ordered_messages(call_info);
    let events = blockifier_to_starknet_rs_ordered_events(&call_info.execution.events);

    let inner_calls = call_info
        .inner_calls
        .iter()
        .map(|call| try_get_funtion_invocation_from_call_info(call, class_hash_cache, block_number))
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
        class_hash.into_field_element()
    } else if let Some(cached_hash) = class_hash_cache.get(&call_info.call.storage_address) {
        *cached_hash
    } else {
        // Compute and cache the class hash
        let Ok(Some(class_hash)) =
            storage_handler::contract_class_hash().get_at(&call_info.call.storage_address, block_number)
        else {
            return Err(TryFuntionInvocationFromCallInfoError::ContractNotFound);
        };

        let computed_hash = FieldElement::from_byte_slice_be(class_hash.0.bytes()).unwrap();
        class_hash_cache.insert(call_info.call.storage_address, computed_hash);

        computed_hash
    };

    // TODO: Replace this with non default exec resources
    let computation_resources = ComputationResources {
        steps: 0,
        memory_holes: None,
        range_check_builtin_applications: None,
        pedersen_builtin_applications: None,
        poseidon_builtin_applications: None,
        ec_op_builtin_applications: None,
        ecdsa_builtin_applications: None,
        bitwise_builtin_applications: None,
        keccak_builtin_applications: None,
        segment_arena_builtin: None,
    };

    Ok(starknet_core::types::FunctionInvocation {
        contract_address: call_info.call.storage_address.0.into_field_element(),
        entry_point_selector: call_info.call.entry_point_selector.0.into_field_element(),
        calldata: call_info.call.calldata.0.iter().map(|x| x.into_field_element()).collect(),
        caller_address: call_info.call.caller_address.0.into_field_element(),
        class_hash,
        entry_point_type,
        call_type,
        result: call_info.execution.retdata.0.iter().map(|x| x.into_field_element()).collect(),
        calls: inner_calls,
        events,
        messages,
        execution_resources: computation_resources,
    })
}

pub fn tx_execution_infos_to_tx_trace(
    tx_type: TxType,
    tx_exec_info: &TransactionExecutionInfo,
    block_number: u64,
) -> Result<TransactionTrace, ConvertCallInfoToExecuteInvocationError> {
    let mut class_hash_cache: HashMap<ContractAddress, FieldElement> = HashMap::new();

    // TODO: Replace this with non default exec resources
    let execution_resources = ExecutionResources {
        computation_resources: ComputationResources {
            steps: 0,
            memory_holes: None,
            range_check_builtin_applications: None,
            pedersen_builtin_applications: None,
            poseidon_builtin_applications: None,
            ec_op_builtin_applications: None,
            ecdsa_builtin_applications: None,
            bitwise_builtin_applications: None,
            keccak_builtin_applications: None,
            segment_arena_builtin: None,
        },
        data_resources: DataResources { data_availability: DataAvailabilityResources { l1_gas: 0, l1_data_gas: 0 } },
    };

    // If simulated with `SimulationFlag::SkipValidate` this will be `None`
    // therefore we cannot unwrap it
    let validate_invocation = tx_exec_info
        .validate_call_info
        .as_ref()
        .map(|call_info| try_get_funtion_invocation_from_call_info(call_info, &mut class_hash_cache, block_number))
        .transpose()?;
    // If simulated with `SimulationFlag::SkipFeeCharge` this will be `None`
    // therefore we cannot unwrap it
    let fee_transfer_invocation = tx_exec_info
        .fee_transfer_call_info
        .as_ref()
        .map(|call_info| try_get_funtion_invocation_from_call_info(call_info, &mut class_hash_cache, block_number))
        .transpose()?;

    let tx_trace = match tx_type {
        TxType::Invoke => TransactionTrace::Invoke(InvokeTransactionTrace {
            validate_invocation,
            execute_invocation: if let Some(e) = &tx_exec_info.revert_error {
                ExecuteInvocation::Reverted(RevertedInvocation { revert_reason: e.clone() })
            } else {
                ExecuteInvocation::Success(try_get_funtion_invocation_from_call_info(
                    // Safe to unwrap because is only `None`  for `Declare` txs
                    tx_exec_info.execute_call_info.as_ref().unwrap(),
                    &mut class_hash_cache,
                    block_number,
                )?)
            },
            fee_transfer_invocation,
            // TODO(#1291): Compute state diff correctly
            state_diff: None,
            execution_resources,
        }),
        TxType::Declare => TransactionTrace::Declare(DeclareTransactionTrace {
            validate_invocation,
            fee_transfer_invocation,
            // TODO(#1291): Compute state diff correctly
            state_diff: None,
            execution_resources,
        }),
        TxType::DeployAccount => {
            TransactionTrace::DeployAccount(DeployAccountTransactionTrace {
                validate_invocation,
                constructor_invocation: try_get_funtion_invocation_from_call_info(
                    // Safe to unwrap because is only `None` for `Declare` txs
                    tx_exec_info.execute_call_info.as_ref().unwrap(),
                    &mut class_hash_cache,
                    block_number,
                )?,
                fee_transfer_invocation,
                // TODO(#1291): Compute state diff correctly
                state_diff: None,
                execution_resources,
            })
        }
        TxType::L1Handler => TransactionTrace::L1Handler(L1HandlerTransactionTrace {
            function_invocation: try_get_funtion_invocation_from_call_info(
                // Safe to unwrap because is only `None` for `Declare` txs
                tx_exec_info.execute_call_info.as_ref().unwrap(),
                &mut class_hash_cache,
                block_number,
            )?,
            state_diff: None,
            execution_resources,
        }),
    };

    Ok(tx_trace)
}

// // TODO: move to mod utils
// pub fn block_number_by_id(id: BlockId) -> Result<u64, StarknetRpcApiError> {
//     let (latest_block_hash, latest_block_number) =
// DeoxysBackend::meta().get_latest_block_hash_and_number()?;     match id {
//         // Check if the block corresponding to the number is stored in the database
//         BlockId::Number(number) => match
// DeoxysBackend::mapping().starknet_block_hash_from_block_number(number)? {             Some(_) =>
// Ok(number),             None => Err(StarknetRpcApiError::BlockNotFound),
//         },
//         BlockId::Hash(block_hash) => {
//             match
// DeoxysBackend::mapping().block_number_from_starknet_block_hash(StarkFelt(block_hash.
// to_bytes_be()))? {                 Some(block_number) => Ok(block_number),
//                 None if block_hash == latest_block_hash => Ok(latest_block_number),
//                 None => Err(StarknetRpcApiError::BlockNotFound),
//             }
//         }
//         BlockId::Tag(BlockTag::Latest) => Ok(latest_block_number),
//         BlockId::Tag(BlockTag::Pending) => Ok(latest_block_number + 1),
//     }
// }
