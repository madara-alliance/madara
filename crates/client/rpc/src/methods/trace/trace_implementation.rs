use std::collections::HashMap;

use blockifier::execution::call_info::CallInfo;
use blockifier::execution::contract_class::{ClassInfo, ContractClass, ContractClassV1};
use blockifier::transaction as btx;
use blockifier::transaction::account_transaction::AccountTransaction;
use blockifier::transaction::objects::TransactionExecutionInfo;
use blockifier::transaction::transactions::L1HandlerTransaction;
use deoxys_runtime::opaque::{DBlockT, DHashT};
use jsonrpsee::core::RpcResult;
use mc_db::DeoxysBackend;
use mc_genesis_data_provider::GenesisProvider;
use mc_storage::StorageOverride;
use mp_block::DeoxysBlock;
use mp_felt::Felt252Wrapper;
use mp_hashers::HasherT;
use mp_transactions::compute_hash::ComputeTransactionHash;
use mp_transactions::{TxType, UserOrL1HandlerTransaction};
use pallet_starknet_runtime_api::{ConvertTransactionRuntimeApi, StarknetRuntimeApi};
use sc_client_api::{Backend, BlockBackend, StorageProvider};
use sc_transaction_pool::ChainApi;
use sc_transaction_pool_api::TransactionPool;
use sp_api::ProvideRuntimeApi;
use sp_blockchain::HeaderBackend;
use sp_runtime::traits::Block as BlockT;
use starknet_api::core::{ClassHash, ContractAddress};
use starknet_api::transaction as stx;
use starknet_core::types::{
    BlockId, DeclareTransactionTrace, DeployAccountTransactionTrace, ExecuteInvocation, ExecutionResources,
    InvokeTransactionTrace, L1HandlerTransactionTrace, RevertedInvocation, TransactionTrace, TransactionTraceWithHash,
};
use starknet_ff::FieldElement;

use super::lib::*;
use crate::errors::StarknetRpcApiError;
use crate::utils::get_block_by_block_hash;
use crate::{Starknet, StarknetReadRpcApiServer};



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

fn try_get_funtion_invocation_from_call_info<B: BlockT>(
    storage_override: &dyn StorageOverride<B>,
    substrate_block_hash: B::Hash,
    call_info: &CallInfo,
    class_hash_cache: &mut HashMap<ContractAddress, FieldElement>,
) -> Result<starknet_core::types::FunctionInvocation, TryFuntionInvocationFromCallInfoError> {
    let messages = collect_call_info_ordered_messages(call_info);
    let events = blockifier_to_starknet_rs_ordered_events(&call_info.execution.events);

    let inner_calls = call_info
        .inner_calls
        .iter()
        .map(|call| {
            try_get_funtion_invocation_from_call_info(storage_override, substrate_block_hash, call, class_hash_cache)
        })
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

    // TODO: Replace this with non default exec resources
    let execution_resources = ExecutionResources {
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
        execution_resources,
    })
}

pub fn tx_execution_infos_to_tx_trace<B: BlockT>(
    storage_override: &dyn StorageOverride<B>,
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
            try_get_funtion_invocation_from_call_info(
                storage_override,
                substrate_block_hash,
                call_info,
                &mut class_hash_cache,
            )
        })
        .transpose()?;
    // If simulated with `SimulationFlag::SkipFeeCharge` this will be `None`
    // therefore we cannot unwrap it
    let fee_transfer_invocation = tx_exec_info
        .fee_transfer_call_info
        .as_ref()
        .map(|call_info| {
            try_get_funtion_invocation_from_call_info(
                storage_override,
                substrate_block_hash,
                call_info,
                &mut class_hash_cache,
            )
        })
        .transpose()?;

    let tx_trace = match tx_type {
        TxType::Invoke => TransactionTrace::Invoke(InvokeTransactionTrace {
            validate_invocation,
            execute_invocation: if let Some(e) = &tx_exec_info.revert_error {
                ExecuteInvocation::Reverted(RevertedInvocation { revert_reason: e.clone() })
            } else {
                ExecuteInvocation::Success(try_get_funtion_invocation_from_call_info(
                    storage_override,
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
                    storage_override,
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
                storage_override,
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
