use blockifier::execution::call_info::CallInfo;
use blockifier::transaction::{
    account_transaction::AccountTransaction,
    objects::{FeeType, GasVector, HasRelatedFeeType, TransactionExecutionInfo},
    transaction_execution::Transaction,
    transactions::L1HandlerTransaction,
};
use cairo_vm::types::builtin_name::BuiltinName;
use starknet_core::types::MsgToL2;
use starknet_types_core::felt::Felt;
use thiserror::Error;

use crate::{
    DeclareTransactionReceipt, DeployAccountTransactionReceipt, Event, ExecutionResources, ExecutionResult, FeePayment,
    InvokeTransactionReceipt, L1Gas, L1HandlerTransactionReceipt, MsgToL1, PriceUnit, TransactionReceipt,
};

fn blockifier_tx_fee_type(tx: &Transaction) -> FeeType {
    match tx {
        Transaction::AccountTransaction(tx) => tx.fee_type(),
        Transaction::L1HandlerTransaction(tx) => tx.fee_type(),
    }
}
fn blockifier_tx_hash(tx: &Transaction) -> Felt {
    match tx {
        Transaction::AccountTransaction(tx) => match tx {
            AccountTransaction::Declare(tx) => tx.tx_hash.0,
            AccountTransaction::DeployAccount(tx) => tx.tx_hash.0,
            AccountTransaction::Invoke(tx) => tx.tx_hash.0,
        },
        Transaction::L1HandlerTransaction(tx) => tx.tx_hash.0,
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

fn get_l1_handler_message_hash(tx: &L1HandlerTransaction) -> Result<Felt, L1HandlerMessageError> {
    let (from_address, payload) = tx.tx.calldata.0.split_first().ok_or(L1HandlerMessageError::EmptyCalldata)?;

    let from_address = (*from_address).try_into().map_err(|_| L1HandlerMessageError::FromAddressOutOfRange)?;

    let nonce = tx.tx.nonce.0.to_bigint().try_into().map_err(|_| L1HandlerMessageError::InvalidNonce)?;

    let message = MsgToL2 {
        from_address,
        to_address: tx.tx.contract_address.into(),
        selector: tx.tx.entry_point_selector.0,
        payload: payload.into(),
        nonce,
    };
    Ok(Felt::from_bytes_le(message.hash().as_bytes()))
}

pub fn from_blockifier_execution_info(res: &TransactionExecutionInfo, tx: &Transaction) -> TransactionReceipt {
    let price_unit = match blockifier_tx_fee_type(tx) {
        FeeType::Eth => PriceUnit::Wei,
        FeeType::Strk => PriceUnit::Fri,
    };

    let actual_fee = FeePayment { amount: res.transaction_receipt.fee.into(), unit: price_unit };
    let transaction_hash = blockifier_tx_hash(tx);

    let mut events: Vec<Event> = Vec::new();
    get_events_from_call_info(res.execute_call_info.as_ref(), 0, &mut events);

    let message_hash = match tx {
        Transaction::L1HandlerTransaction(tx) => match get_l1_handler_message_hash(tx) {
            Ok(hash) => Some(hash),
            Err(err) => {
                log::error!("Error getting l1 handler message hash: {:?}", err);
                None
            }
        },
        _ => None,
    };

    let messages_sent = res
        .non_optional_call_infos()
        .flat_map(|call| {
            call.execution.l2_to_l1_messages.iter().map(|message| MsgToL1 {
                from_address: call.call.storage_address.into(),
                to_address: message.message.to_address.into(),
                payload: message.message.payload.0.clone(),
            })
        })
        .collect();

    let get_applications = |resource| {
        res.non_optional_call_infos()
            .map(|call| call.resources.builtin_instance_counter.get(resource).map(|el| *el as u64))
            .sum()
    };

    let memory_holes = res.non_optional_call_infos().map(|call| call.resources.n_memory_holes as u64).sum();

    let execution_resources = ExecutionResources {
        steps: res.non_optional_call_infos().map(|call| call.resources.n_steps as u64).sum(),
        memory_holes: if memory_holes == 0 { None } else { Some(memory_holes) },
        range_check_builtin_applications: get_applications(&BuiltinName::range_check),
        pedersen_builtin_applications: get_applications(&BuiltinName::pedersen),
        poseidon_builtin_applications: get_applications(&BuiltinName::poseidon),
        ec_op_builtin_applications: get_applications(&BuiltinName::ec_op),
        ecdsa_builtin_applications: get_applications(&BuiltinName::ecdsa),
        bitwise_builtin_applications: get_applications(&BuiltinName::bitwise),
        keccak_builtin_applications: get_applications(&BuiltinName::keccak),
        segment_arena_builtin: get_applications(&BuiltinName::segment_arena),
        data_availability: res.transaction_receipt.da_gas.into(),
        total_gas_consumed: res.transaction_receipt.gas.into(),
    };

    let execution_result = if let Some(reason) = &res.revert_error {
        ExecutionResult::Reverted { reason: reason.into() }
    } else {
        ExecutionResult::Succeeded
    };

    match tx {
        Transaction::AccountTransaction(AccountTransaction::Declare(_)) => {
            TransactionReceipt::Declare(DeclareTransactionReceipt {
                transaction_hash,
                actual_fee,
                messages_sent,
                events,
                execution_resources,
                execution_result,
            })
        }
        Transaction::AccountTransaction(AccountTransaction::DeployAccount(tx)) => {
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
        Transaction::AccountTransaction(AccountTransaction::Invoke(_)) => {
            TransactionReceipt::Invoke(InvokeTransactionReceipt {
                transaction_hash,
                actual_fee,
                messages_sent,
                events,
                execution_resources,
                execution_result,
            })
        }
        Transaction::L1HandlerTransaction(_tx) => TransactionReceipt::L1Handler(L1HandlerTransactionReceipt {
            transaction_hash,
            actual_fee,
            messages_sent,
            events,
            execution_resources,
            execution_result,
            message_hash: message_hash.unwrap(), // it's a safe unwrap because it would've panicked earlier if it was Err
        }),
    }
}

impl From<GasVector> for L1Gas {
    fn from(value: GasVector) -> Self {
        L1Gas { l1_gas: value.l1_gas as _, l1_data_gas: value.l1_data_gas as _ }
    }
}

/// To get all the events from the CallInfo including the inner call events.
fn get_events_from_call_info(call_info: Option<&CallInfo>, next_order: usize, events_vec: &mut Vec<Event>) -> usize {
    let mut event_idx = 0;
    let mut inner_call_idx = 0;
    let mut next_order = next_order;

    let call_info = match call_info {
        Some(call) => call,
        None => return 0,
    };

    loop {
        if event_idx < call_info.execution.events.len() {
            let ordered_event = &call_info.execution.events[event_idx];
            if ordered_event.order == next_order {
                let event = Event {
                    from_address: call_info.call.storage_address.into(),
                    keys: ordered_event.event.keys.iter().map(|k| k.0).collect(),
                    data: ordered_event.event.data.0.clone(),
                };
                events_vec.push(event);
                next_order += 1;
                event_idx += 1;
                continue;
            }
        }

        if inner_call_idx < call_info.inner_calls.len() {
            next_order =
                get_events_from_call_info(Some(&call_info.inner_calls[inner_call_idx]), next_order, events_vec);
            inner_call_idx += 1;
            continue;
        }

        break;
    }

    next_order
}

#[cfg(test)]
mod events_logic_tests {
    use crate::from_blockifier::get_events_from_call_info;
    use crate::Event;
    use blockifier::execution::call_info::{CallExecution, CallInfo, OrderedEvent};
    use rstest::rstest;
    use starknet_api::transaction::{EventContent, EventData, EventKey};
    use starknet_types_core::felt::Felt;

    #[rstest]
    fn test_event_ordering() {
        let mut events = Vec::new();
        let nested_calls = create_call_info(
            0,
            vec![create_call_info(
                1,
                vec![create_call_info(2, vec![create_call_info(3, vec![create_call_info(4, vec![])])])],
            )],
        );
        get_events_from_call_info(Some(&nested_calls), 0, &mut events);

        let expected_events_ordering = vec![event(0), event(1), event(2), event(3), event(4)];

        assert_eq!(expected_events_ordering, events);
    }

    fn create_call_info(event_number: u32, inner_calls: Vec<CallInfo>) -> CallInfo {
        CallInfo {
            call: Default::default(),
            execution: execution(vec![ordered_event(event_number as usize)]),
            resources: Default::default(),
            inner_calls,
            storage_read_values: vec![],
            accessed_storage_keys: Default::default(),
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
