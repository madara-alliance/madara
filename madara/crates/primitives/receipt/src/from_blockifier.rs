use crate::{
    DeclareTransactionReceipt, DeployAccountTransactionReceipt, Event, ExecutionResources, ExecutionResult, FeePayment,
    GasVector, InvokeTransactionReceipt, L1HandlerTransactionReceipt, MsgToL1, MsgToL2, PriceUnit, TransactionReceipt,
};
use anyhow::anyhow;
use blockifier::execution::call_info::CallInfo;
use blockifier::transaction::{
    account_transaction::AccountTransaction as BlockifierAccountTransaction,
    objects::{HasRelatedFeeType, TransactionExecutionInfo},
    transaction_execution::Transaction,
};
use cairo_vm::types::builtin_name::BuiltinName;
use starknet_api::block::FeeType;
use starknet_api::executable_transaction::AccountTransaction as ApiAccountTransaction;
use starknet_api::execution_resources::GasVector as ApiGasVector;
use starknet_api::transaction::L1HandlerTransaction;
use starknet_core::types::Hash256;
use starknet_types_core::felt::Felt;
use std::convert::TryFrom;
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

impl TryFrom<&mp_transactions::L1HandlerTransaction> for MsgToL2 {
    type Error = anyhow::Error;

    fn try_from(tx: &mp_transactions::L1HandlerTransaction) -> Result<Self, Self::Error> {
        let (from_address, payload) = tx.calldata.split_first().ok_or_else(|| anyhow!("Empty calldata"))?;

        Ok(Self {
            from_address: *from_address,
            to_address: tx.contract_address,
            selector: tx.entry_point_selector,
            payload: payload.to_vec(),
            nonce: Some(tx.nonce.into()),
        })
    }
}

fn get_l1_handler_message_hash(tx: &L1HandlerTransaction) -> Result<Hash256, L1HandlerMessageError> {
    let (from_address, payload) = tx.calldata.0.split_first().ok_or(L1HandlerMessageError::EmptyCalldata)?;

    let nonce = Some(tx.nonce.0);

    let message = MsgToL2 {
        from_address: *from_address,
        to_address: tx.contract_address.into(),
        selector: tx.entry_point_selector.0,
        payload: payload.into(),
        nonce,
    };
    Ok(message.compute_hash())
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

    // get all validate_calls
    let validate_calls = res.validate_call_info.iter().flat_map(|call_info| call_info.iter());

    let validate_events = validate_calls
        .flat_map(|call| {
            call.execution.events.iter().map(|event| {
                let ordered_event = Event {
                    // See above for why we use storage address.
                    from_address: call.call.storage_address.into(),
                    keys: event.event.keys.iter().map(|k| k.0).collect(),
                    data: event.event.data.0.clone(),
                };
                (ordered_event, event.order)
            })
        }).collect::<Vec<(Event, usize)>>();

    // Sort by order and extract just the Events
    let mut validate_events_with_order = validate_events;
    validate_events_with_order.sort_by_key(|(_, order)| *order);
    let final_validate_events: Vec<Event> = validate_events_with_order.into_iter().map(|(event, _)| event).collect();

    // get all execute_calls
    let execute_calls = res.execute_call_info.iter().flat_map(|call_info| call_info.iter());

    let execute_events = execute_calls
        .flat_map(|call| {
            call.execution.events.iter().map(|event| {
                let ordered_event = Event {
                    // See above for why we use storage address.
                    from_address: call.call.storage_address.into(),
                    keys: event.event.keys.iter().map(|k| k.0).collect(),
                    data: event.event.data.0.clone(),
                };
                (ordered_event, event.order)
            })
        }).collect::<Vec<(Event, usize)>>();

    // Sort by order and extract just the Events
    let mut execute_events_with_order = execute_events;
    execute_events_with_order.sort_by_key(|(_, order)| *order);
    let final_execute_events: Vec<Event> = execute_events_with_order.into_iter().map(|(event, _)| event).collect();


    // get all fee_transfer_calls
    let fee_transfer_calls = res.fee_transfer_call_info.iter().flat_map(|call_info| call_info.iter());

    let fee_transfer_events = fee_transfer_calls
        .flat_map(|call| {
            call.execution.events.iter().map(|event| {
                let ordered_event = Event {
                    // See above for why we use storage address.
                    from_address: call.call.storage_address.into(),
                    keys: event.event.keys.iter().map(|k| k.0).collect(),
                    data: event.event.data.0.clone(),
                };
                (ordered_event, event.order)
            })
        }).collect::<Vec<(Event, usize)>>();

    // Sort by order and extract just the Events
    let mut fee_transfer_events_with_order = fee_transfer_events;
    fee_transfer_events_with_order.sort_by_key(|(_, order)| *order);
    let final_fee_transfer_events: Vec<Event> = fee_transfer_events_with_order.into_iter().map(|(event, _)| event).collect();


    let mut events = final_validate_events.clone();
    events.extend(final_execute_events);
    events.extend(final_fee_transfer_events);

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



impl From<ApiGasVector> for GasVector {
    fn from(value: ApiGasVector) -> Self {
        GasVector {
            l1_gas: value.l1_gas.0.into(),
            l1_data_gas: value.l1_data_gas.0.into(),
            l2_gas: value.l2_gas.0.into(),
        }
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
        CallInfo { execution: execution(vec![ordered_event(event_number as usize)]), inner_calls, ..Default::default() }
    }

    fn execution(events: Vec<OrderedEvent>) -> CallExecution {
        CallExecution {
            retdata: Default::default(),
            events,
            l2_to_l1_messages: vec![],
            cairo_native: false,
            failed: false,
            gas_consumed: Default::default(),
            cairo_native: false,
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

#[cfg(test)]
mod test {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn test_compute_hash_msg_to_l2() {
        let msg = MsgToL2 {
            from_address: Felt::from(1),
            to_address: Felt::from(2),
            selector: Felt::from(3),
            payload: vec![Felt::from(4), Felt::from(5), Felt::from(6)],
            nonce: Some(Felt::from(7)),
        };

        let hash = msg.compute_hash();

        let expected_hash =
            Hash256::from_str("0xeec1e25e91757d5e9c8a11cf6e84ddf078dbfbee23382ee979234fc86a8608a5").unwrap();

        assert_eq!(hash, expected_hash);
    }
}
