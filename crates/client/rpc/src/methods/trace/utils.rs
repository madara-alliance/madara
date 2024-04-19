use std::collections::HashMap;

use blockifier::execution::call_info::CallInfo;
use blockifier::execution::contract_class::{ClassInfo, ContractClass, ContractClassV1};
use blockifier::transaction as btx;
use blockifier::transaction::account_transaction::AccountTransaction;
use blockifier::transaction::objects::TransactionExecutionInfo;
use blockifier::transaction::transaction_execution::Transaction;
use blockifier::transaction::transactions::L1HandlerTransaction;
use mc_db::DeoxysBackend;
use mc_storage::StorageOverride;
use mp_block::DeoxysBlock;
use mp_felt::Felt252Wrapper;
use mp_hashers::HasherT;
use mp_transactions::compute_hash::ComputeTransactionHash;
use mp_transactions::TxType;
use mp_types::block::{DBlockT, DHashT};
use sc_client_api::{Backend, BlockBackend, StorageProvider};
use sc_transaction_pool::ChainApi;
use sp_blockchain::HeaderBackend;
use sp_runtime::traits::Block as BlockT;
use starknet_api::core::{ClassHash, ContractAddress};
use starknet_api::transaction as stx;
use starknet_core::types::{
    ComputationResources, DataAvailabilityResources, DataResources, DeclareTransactionTrace,
    DeployAccountTransactionTrace, ExecuteInvocation, ExecutionResources, InvokeTransactionTrace,
    L1HandlerTransactionTrace, RevertedInvocation, TransactionTrace,
};
use starknet_ff::FieldElement;

use super::lib::*;
use crate::errors::StarknetRpcApiError;
use crate::Starknet;

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

pub fn try_get_funtion_invocation_from_call_info<B: BlockT>(
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
        execution_resources: computation_resources,
    })
}

pub fn tx_execution_infos_to_tx_trace<B: BlockT>(
    storage_override: &dyn StorageOverride<B>,
    substrate_block_hash: B::Hash,
    tx_type: TxType,
    tx_exec_info: &TransactionExecutionInfo,
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
                    storage_override,
                    substrate_block_hash,
                    // Safe to unwrap because is only `None` for `Declare` txs
                    tx_exec_info.execute_call_info.as_ref().unwrap(),
                    &mut class_hash_cache,
                )?,
                fee_transfer_invocation,
                // TODO(#1291): Compute state diff correctly
                state_diff: None,
                execution_resources,
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
            execution_resources,
        }),
    };

    Ok(tx_trace)
}

pub(crate) fn map_transaction_to_user_transaction<A, BE, G, C, P, H>(
    starknet: &Starknet<A, BE, G, C, P, H>,
    starknet_block: DeoxysBlock,
    substrate_block_hash: DHashT,
    chain_id: Felt252Wrapper,
    target_transaction_hash: Option<Felt252Wrapper>,
) -> Result<(Vec<Transaction>, Vec<Transaction>), StarknetRpcApiError>
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

        if Some(Felt252Wrapper::from(current_tx_hash)) == target_transaction_hash {
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
    tx: &stx::Transaction,
    starknet: &Starknet<A, BE, G, C, P, H>,
    substrate_block_hash: DHashT,
    chain_id: Felt252Wrapper,
    block_number: u64,
) -> Result<Transaction, StarknetRpcApiError>
where
    A: ChainApi<Block = DBlockT> + 'static,
    C: HeaderBackend<DBlockT> + BlockBackend<DBlockT> + StorageProvider<DBlockT, BE> + 'static,
    H: HasherT + Send + Sync + 'static,
    BE: Backend<DBlockT> + 'static,
{
    match tx {
        stx::Transaction::Invoke(invoke_tx) => {
            let tx = btx::transactions::InvokeTransaction {
                tx: invoke_tx.clone(),
                tx_hash: invoke_tx.compute_hash::<H>(chain_id, false, Some(block_number)),
                // TODO: Check if this is correct
                only_query: false,
            };
            Ok(Transaction::AccountTransaction(AccountTransaction::Invoke(tx)))
        }
        stx::Transaction::DeployAccount(deploy_account_tx) => {
            let tx = btx::transactions::DeployAccountTransaction {
                tx: deploy_account_tx.clone(),
                tx_hash: deploy_account_tx.compute_hash::<H>(chain_id, false, Some(block_number)),
                // TODO: Fill this with non default contract address
                contract_address: ContractAddress::default(),
                // TODO: Check if this is correct
                only_query: false,
            };
            Ok(Transaction::AccountTransaction(AccountTransaction::DeployAccount(tx)))
        }
        stx::Transaction::Declare(declare_tx) => {
            let class_hash = ClassHash::from(Felt252Wrapper::from(*declare_tx.class_hash()));

            match declare_tx {
                stx::DeclareTransaction::V0(_) | stx::DeclareTransaction::V1(_) => {
                    let contract_class = starknet
                        .overrides
                        .for_block_hash(starknet.client.as_ref(), substrate_block_hash)
                        .contract_class_by_class_hash(substrate_block_hash, class_hash)
                        .ok_or_else(|| {
                            log::error!("Failed to retrieve contract class from hash '{class_hash}'");
                            StarknetRpcApiError::InternalServerError
                        })?;

                    // TODO: fix class info declaration with non defaulted values
                    let class_info = ClassInfo::new(&contract_class, 10, 10).unwrap();

                    let tx = btx::transactions::DeclareTransaction::new(
                        declare_tx.clone(),
                        declare_tx.compute_hash::<H>(chain_id, false, Some(block_number)),
                        class_info,
                    )
                    .unwrap();

                    Ok(Transaction::AccountTransaction(AccountTransaction::Declare(tx)))
                }
                stx::DeclareTransaction::V2(_tx) => {
                    // TODO: change this contract class to the correct one
                    let contract_class = starknet_api::state::ContractClass::default();
                    let contract_class = mp_transactions::utils::sierra_to_casm_contract_class(contract_class)
                        .map_err(|e| {
                            log::error!("Failed to convert the SierraContractClass to CasmContractClass: {e}");
                            StarknetRpcApiError::InternalServerError
                        })?;
                    let contract_class = ContractClass::V1(ContractClassV1::try_from(contract_class).map_err(|e| {
                        log::error!(
                            "Failed to convert the compiler CasmContractClass to blockifier CasmContractClass: {e}"
                        );
                        StarknetRpcApiError::InternalServerError
                    })?);

                    // TODO: fix class info declaration with non defaulted values
                    let class_info = ClassInfo::new(&contract_class, 10, 10).unwrap();

                    let tx = btx::transactions::DeclareTransaction::new(
                        declare_tx.clone(),
                        declare_tx.compute_hash::<H>(chain_id, false, Some(block_number)),
                        class_info,
                    )
                    .unwrap();

                    Ok(Transaction::AccountTransaction(AccountTransaction::Declare(tx)))
                }
                stx::DeclareTransaction::V3(_) => todo!(),
            }
        }
        stx::Transaction::L1Handler(handle_l1_message_tx) => {
            let tx_hash = handle_l1_message_tx.compute_hash::<H>(chain_id, false, Some(block_number));
            let paid_fee_on_l1: starknet_api::transaction::Fee = DeoxysBackend::l1_handler_paid_fee()
                .get_fee_paid_for_l1_handler_tx(Felt252Wrapper::from(tx_hash).into())
                .map_err(|e| {
                    log::error!("Failed to retrieve fee paid on l1 for tx with hash `{tx_hash:?}`: {e}");
                    StarknetRpcApiError::InternalServerError
                })?;

            Ok(Transaction::L1HandlerTransaction(L1HandlerTransaction {
                tx: handle_l1_message_tx.clone(),
                tx_hash,
                paid_fee_on_l1,
            }))
        }
        stx::Transaction::Deploy(_) => todo!(),
    }
}
