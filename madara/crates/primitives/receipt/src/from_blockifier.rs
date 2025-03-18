use crate::{
    DeclareTransactionReceipt, DeployAccountTransactionReceipt, Event, ExecutionResources, ExecutionResult, FeePayment,
    InvokeTransactionReceipt, L1Gas, L1HandlerTransactionReceipt, MsgToL1, PriceUnit, TransactionReceipt,
};
use blockifier::execution::call_info::CallInfo;
use blockifier::transaction::{
    account_transaction::AccountTransaction as BlockifierAccountTransaction,
    objects::{HasRelatedFeeType, TransactionExecutionInfo},
    transaction_execution::Transaction,
};
use cairo_vm::types::builtin_name::BuiltinName;
use starknet_api::block::FeeType;
use starknet_api::executable_transaction::AccountTransaction as ApiAccountTransaction;
use starknet_api::execution_resources::GasVector;
use starknet_api::transaction::L1HandlerTransaction;
use starknet_core::types::{Hash256, MsgToL2};
use starknet_types_core::felt::Felt;
use thiserror::Error;

fn blockifier_tx_fee_type(tx: &Transaction) -> FeeType {
    match tx {
        Transaction::Account(tx) => tx.fee_type(),
        Transaction::L1Handler(tx) => tx.fee_type(),
    }
}
fn blockifier_tx_hash(tx: &Transaction) -> Felt {
    match tx {
        Transaction::Account(tx) => *tx.tx_hash(),
        Transaction::L1Handler(tx) => tx.tx_hash.0,
    }
}

#[derive(Debug, Error)]
pub enum L1HandlerMessageError {
    #[error("Empty calldata")]
    EmptyCalldata,
    #[error("From address out of range")]
    FromAddressOutOfRange,
    #[error("Invalid nonce")]
    InvalidNonce,
}

fn get_l1_handler_message_hash(tx: &L1HandlerTransaction) -> Result<Hash256, L1HandlerMessageError> {
    let (from_address, payload) = tx.calldata.0.split_first().ok_or(L1HandlerMessageError::EmptyCalldata)?;

    let from_address = (*from_address).try_into().map_err(|_| L1HandlerMessageError::FromAddressOutOfRange)?;

    let nonce = tx.nonce.0.to_bigint().try_into().map_err(|_| L1HandlerMessageError::InvalidNonce)?;

    let message = MsgToL2 {
        from_address,
        to_address: tx.contract_address.into(),
        selector: tx.entry_point_selector.0,
        payload: payload.into(),
        nonce,
    };
    Ok(message.hash())
}

fn recursive_call_info_iter(res: &TransactionExecutionInfo) -> impl Iterator<Item = &CallInfo> {
    res
        .non_optional_call_infos() // all root callinfos
        .flat_map(|call_info| call_info.iter()) // flatmap over the roots' recursive inner call infos
}

pub fn from_blockifier_execution_info(res: &TransactionExecutionInfo, tx: &Transaction) -> TransactionReceipt {
    let price_unit = match blockifier_tx_fee_type(tx) {
        FeeType::Eth => PriceUnit::Wei,
        FeeType::Strk => PriceUnit::Fri,
    };

    let actual_fee = FeePayment { amount: res.receipt.fee.into(), unit: price_unit };
    let transaction_hash = blockifier_tx_hash(tx);

    let messages_sent = recursive_call_info_iter(res)
        .flat_map(|call| {
            call.execution.l2_to_l1_messages.iter().map(|message| MsgToL1 {
                // Note: storage address here to identify the contract. Not caller address nor code address, because of delegate (library) calls.
                from_address: call.call.storage_address.into(),
                to_address: message.message.to_address.into(),
                payload: message.message.payload.0.clone(),
            })
        })
        .collect();
    let events = recursive_call_info_iter(res)
        .flat_map(|call| {
            call.execution.events.iter().map(|event| Event {
                // See above for why we use storage address.
                from_address: call.call.storage_address.into(),
                keys: event.event.keys.iter().map(|k| k.0).collect(),
                data: event.event.data.0.clone(),
            })
        })
        .collect();

    // Note: these should not be iterated over recursively because they include the inner calls
    // We only add up the root calls here without recursing into the inner calls.

    let get_applications = |resource| {
        res.non_optional_call_infos()
            .map(|call| call.resources.builtin_instance_counter.get(resource).map(|el| *el as u64))
            .sum::<Option<_>>()
            .unwrap_or_default()
    };

    let memory_holes = res.non_optional_call_infos().map(|call| call.resources.n_memory_holes as u64).sum();

    let execution_resources = ExecutionResources {
        steps: res.non_optional_call_infos().map(|call| call.resources.n_steps as u64).sum(),
        memory_holes,
        range_check_builtin_applications: get_applications(&BuiltinName::range_check),
        pedersen_builtin_applications: get_applications(&BuiltinName::pedersen),
        poseidon_builtin_applications: get_applications(&BuiltinName::poseidon),
        ec_op_builtin_applications: get_applications(&BuiltinName::ec_op),
        ecdsa_builtin_applications: get_applications(&BuiltinName::ecdsa),
        bitwise_builtin_applications: get_applications(&BuiltinName::bitwise),
        keccak_builtin_applications: get_applications(&BuiltinName::keccak),
        segment_arena_builtin: get_applications(&BuiltinName::segment_arena),
        data_availability: res.receipt.da_gas.into(),
        total_gas_consumed: res.receipt.gas.into(),
    };

    let execution_result = if let Some(reason) = &res.revert_error {
        ExecutionResult::Reverted { reason: reason.to_string() }
    } else {
        ExecutionResult::Succeeded
    };

    match tx {
        Transaction::Account(BlockifierAccountTransaction { tx: ApiAccountTransaction::Declare(_), .. }) => {
            TransactionReceipt::Declare(DeclareTransactionReceipt {
                transaction_hash,
                actual_fee,
                messages_sent,
                events,
                execution_resources,
                execution_result,
            })
        }
        Transaction::Account(BlockifierAccountTransaction { tx: ApiAccountTransaction::DeployAccount(tx), .. }) => {
            TransactionReceipt::DeployAccount(DeployAccountTransactionReceipt {
                transaction_hash,
                actual_fee,
                messages_sent,
                events,
                execution_resources,
                execution_result,
                contract_address: tx.contract_address.into(),
            })
        }
        Transaction::Account(BlockifierAccountTransaction { tx: ApiAccountTransaction::Invoke(_), .. }) => {
            TransactionReceipt::Invoke(InvokeTransactionReceipt {
                transaction_hash,
                actual_fee,
                messages_sent,
                events,
                execution_resources,
                execution_result,
            })
        }
        Transaction::L1Handler(tx) => TransactionReceipt::L1Handler(L1HandlerTransactionReceipt {
            transaction_hash,
            actual_fee,
            messages_sent,
            events,
            execution_resources,
            execution_result,
            // This should not panic unless blockifier gives a garbage receipt.
            // TODO: we should have a soft error here just in case.
            message_hash: get_l1_handler_message_hash(&tx.tx).expect("Error getting l1 handler message hash"),
        }),
    }
}

impl From<GasVector> for L1Gas {
    fn from(value: GasVector) -> Self {
        L1Gas { l1_gas: value.l1_gas.0 as _, l1_data_gas: value.l1_data_gas.0 as _ }
    }
}

#[cfg(test)]
mod events_logic_tests {
    use super::*;
    use crate::Event;
    use blockifier::execution::call_info::{CallExecution, CallInfo, OrderedEvent};
    use rstest::rstest;
    use starknet_api::transaction::{EventContent, EventData, EventKey};
    use starknet_types_core::felt::Felt;

    #[rstest]
    fn test_event_ordering() {
        let nested_calls = create_call_info(
            0,
            vec![create_call_info(
                1,
                vec![create_call_info(2, vec![create_call_info(3, vec![create_call_info(4, vec![])])])],
            )],
        );
        let call_2 = create_call_info(5, vec![]);
        let events: Vec<_> = recursive_call_info_iter(&TransactionExecutionInfo {
            validate_call_info: Some(nested_calls),
            execute_call_info: None,
            fee_transfer_call_info: Some(call_2),
            revert_error: None,
            receipt: Default::default(),
        })
        .flat_map(|call| {
            call.execution.events.iter().map(|event| Event {
                // See above for why we use storage address.
                from_address: call.call.storage_address.into(),
                keys: event.event.keys.iter().map(|k| k.0).collect(),
                data: event.event.data.0.clone(),
            })
        })
        .collect();
        let expected_events_ordering = vec![event(0), event(1), event(2), event(3), event(4), event(5)];

        assert_eq!(expected_events_ordering, events);
    }

    fn create_call_info(event_number: u32, inner_calls: Vec<CallInfo>) -> CallInfo {
        CallInfo {
            execution: execution(vec![ordered_event(event_number as usize)]),
            inner_calls,
            storage_read_values: vec![],
            ..Default::default()
        }
    }

    fn execution(events: Vec<OrderedEvent>) -> CallExecution {
        CallExecution {
            retdata: Default::default(),
            events,
            l2_to_l1_messages: vec![],
            failed: false,
            gas_consumed: Default::default(),
        }
    }

    fn ordered_event(order: usize) -> OrderedEvent {
        OrderedEvent {
            order,
            event: EventContent { keys: vec![EventKey(Felt::ZERO); order], data: EventData(vec![Felt::ZERO; order]) },
        }
    }

    fn event(order: usize) -> Event {
        Event { from_address: Default::default(), keys: vec![Felt::ZERO; order], data: vec![Felt::ZERO; order] }
    }
}
