use crate::{
    DeclareTransactionReceipt, DeployAccountTransactionReceipt, Event, ExecutionResources, ExecutionResult, FeePayment,
    GasVector, InvokeTransactionReceipt, L1HandlerTransactionReceipt, MsgToL1, MsgToL2, PriceUnit, TransactionReceipt,
};
use anyhow::anyhow;
use blockifier::execution::call_info::CallInfo;
use blockifier::execution::stack_trace::{ErrorStack, ErrorStackSegment, PreambleType};
use blockifier::transaction::{
    account_transaction::AccountTransaction as BlockifierAccountTransaction,
    objects::{HasRelatedFeeType, RevertError, TransactionExecutionInfo},
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

/// Extension trait for `RevertError` to provide proper formatting with filtered VM tracebacks.
///
/// This trait implements the `format_for_receipt()` method that was removed from blockifier.
/// It filters redundant VM tracebacks from error stacks to make error messages more readable.
pub trait RevertErrorExt {
    /// Returns a new RevertError with filtered VM tracebacks.
    ///
    /// This method leverages the typed structure of `RevertError`. For execution errors,
    /// it filters redundant VM tracebacks from the error stack, keeping only the most
    /// relevant ones. For post-execution errors, it returns a clone of the original error.
    ///
    /// # Returns
    /// A new `RevertError` with filtered error information.
    fn format_for_receipt(&self) -> RevertError;
}

impl RevertErrorExt for RevertError {
    fn format_for_receipt(&self) -> RevertError {
        match self {
            RevertError::Execution(error_stack) => {
                // Create a new ErrorStack with filtered segments
                let filtered_segments = filter_redundant_vm_tracebacks(error_stack);
                let mut new_stack = ErrorStack {
                    header: error_stack.header.clone(),
                    stack: Vec::new(),
                };

                // Clone the filtered segments into the new stack
                for segment in filtered_segments {
                    new_stack.push(segment.clone());
                }

                RevertError::Execution(new_stack)
            }
            RevertError::PostExecution(fee_error) => RevertError::PostExecution(fee_error.clone()),
        }
    }
}

/// Filters redundant VM tracebacks from the error stack.
///
/// The blockifier generates VM tracebacks at every level of the call stack.
/// This function filters them to show the traceback only once, positioned after
/// the last regular contract call (CallContract) entry point frame and before
/// any library call (LibraryCall) frames or the final error.
///
/// Rules:
/// - Always keep VM tracebacks that belong to LibraryCall entries
/// - For CallContract entries:
///   - Keep the traceback if the next entry is a LibraryCall or if it's the last entry
///   - Remove the traceback if the next entry is another CallContract
fn filter_redundant_vm_tracebacks(error_stack: &ErrorStack) -> Vec<&ErrorStackSegment> {
    let mut filtered = Vec::new();
    let len = error_stack.stack.len();

    for i in 0..len {
        let segment = &error_stack.stack[i];

        match segment {
            ErrorStackSegment::Vm(_) => {
                // Find the owning EntryPoint by looking backward
                let owning_entry = find_parent_entry_point(error_stack, i);

                let should_keep = match owning_entry {
                    Some(entry_point) => {
                        // Always keep VM tracebacks for LibraryCall entries
                        if entry_point.preamble_type == PreambleType::LibraryCall {
                            true
                        } else {
                            // For CallContract entries, check what comes next
                            !has_nested_call_contract_after(error_stack, i)
                        }
                    }
                    // If we can't find an owning entry, keep the traceback
                    None => true,
                };

                if should_keep {
                    filtered.push(segment);
                }
            }
            // Always keep non-VM segments
            _ => filtered.push(segment),
        }
    }

    filtered
}

/// Finds the EntryPoint that owns the VM traceback at the given index.
/// Looks backward from the current index to find the most recent EntryPoint.
fn find_parent_entry_point(error_stack: &ErrorStack, vm_index: usize) -> Option<&blockifier::execution::stack_trace::EntryPointErrorFrame> {
    for i in (0..vm_index).rev() {
        if let ErrorStackSegment::EntryPoint(entry_point) = &error_stack.stack[i] {
            return Some(entry_point);
        }
    }
    None
}

/// Checks if there's a nested CallContract entry after the given VM traceback index.
/// Returns true if the next EntryPoint is a CallContract, false otherwise.
fn has_nested_call_contract_after(error_stack: &ErrorStack, vm_index: usize) -> bool {
    // Scan forward to find the next EntryPoint
    for segment in error_stack.stack.iter().skip(vm_index + 1) {
        if let ErrorStackSegment::EntryPoint(entry_point) = segment {
            // Found the next entry point - check if it's a CallContract
            return entry_point.preamble_type == PreambleType::CallContract;
        }
    }
    // No next EntryPoint found, so this is the last one - keep the traceback
    false
}

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
        let formatted_reason = reason.format_for_receipt();
        ExecutionResult::Reverted { reason: formatted_reason.to_string() }
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
    use blockifier::execution::stack_trace::{
        EntryPointErrorFrame, ErrorStackHeader, VmExceptionFrame,
    };
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

    #[test]
    fn test_format_for_receipt_filters_redundant_tracebacks() {
        use cairo_vm::types::relocatable::Relocatable;
        use starknet_api::core::EntryPointSelector;
        use starknet_api::{class_hash, contract_address, felt};

        // Build the ErrorStack structure that represents the unfiltered error
        let mut error_stack = ErrorStack {
            header: ErrorStackHeader::Execution,
            stack: vec![],
        };

        // Entry 0: CallContract with VM traceback (should be removed)
        error_stack.push(
            EntryPointErrorFrame {
                depth: 0,
                preamble_type: PreambleType::CallContract,
                storage_address: contract_address!("0x05743c833ed33a3433f1c5587dac97753ddcc84f9844e6fa2a3268e5ae35cbc3"),
                class_hash: class_hash!("0x073414441639dcd11d1846f287650a00c60c416b9d3ba45d31c651672125b2c2"),
                selector: Some(EntryPointSelector(felt!("0x015d40a3d6ca2ac30f4031e42be28da9b056fef9bb7357ac5e85627ee876e5ad"))),
            }
            .into(),
        );
        error_stack.push(
            VmExceptionFrame {
                pc: Relocatable { segment_index: 0, offset: 35988 },
                error_attr_value: None,
                traceback: Some("Cairo traceback (most recent call last):\nUnknown location (pc=0:330)\nUnknown location (pc=0:11695)\nUnknown location (pc=0:36001)\nUnknown location (pc=0:36001)\nUnknown location (pc=0:36001)\n".to_string()),
            }
            .into(),
        );

        // Entry 1: CallContract with VM traceback (should be removed)
        error_stack.push(
            EntryPointErrorFrame {
                depth: 1,
                preamble_type: PreambleType::CallContract,
                storage_address: contract_address!("0x0286003f7c7bfc3f94e8f0af48b48302e7aee2fb13c23b141479ba00832ef2c6"),
                class_hash: class_hash!("0x03e283b1e8bce178469acb94700999ecc7ad180420201e16eb0a81294ae8599b"),
                selector: Some(EntryPointSelector(felt!("0x0056878e39e16b42520b0d7936d3fd3498f86ceda4dbad50f6ff717644c95ed6"))),
            }
            .into(),
        );
        error_stack.push(
            VmExceptionFrame {
                pc: Relocatable { segment_index: 0, offset: 115867 },
                error_attr_value: None,
                traceback: Some("Cairo traceback (most recent call last):\nUnknown location (pc=0:9435)\nUnknown location (pc=0:43555)\nUnknown location (pc=0:93296)\n".to_string()),
            }
            .into(),
        );

        // Entry 2: CallContract with VM traceback (should be kept - last before LibraryCall)
        error_stack.push(
            EntryPointErrorFrame {
                depth: 2,
                preamble_type: PreambleType::CallContract,
                storage_address: contract_address!("0x06f373b346561036d98ea10fb3e60d2f459c872b1933b50b21fe6ef4fda3b75e"),
                class_hash: class_hash!("0x070cdfaea3ec997bd3a8cdedfc0ffe804a58afc3d6b5a6e5c0218ec233ceea6d"),
                selector: Some(EntryPointSelector(felt!("0x0041b033f4a31df8067c24d1e9b550a2ce75fd4a29e1147af9752174f0e6cb20"))),
            }
            .into(),
        );
        error_stack.push(
            VmExceptionFrame {
                pc: Relocatable { segment_index: 0, offset: 32 },
                error_attr_value: None,
                traceback: Some("Cairo traceback (most recent call last):\nUnknown location (pc=0:1683)\nUnknown location (pc=0:1669)\n".to_string()),
            }
            .into(),
        );

        // Entry 3: LibraryCall with final error
        error_stack.push(
            EntryPointErrorFrame {
                depth: 3,
                preamble_type: PreambleType::LibraryCall,
                storage_address: contract_address!("0x06f373b346561036d98ea10fb3e60d2f459c872b1933b50b21fe6ef4fda3b75e"),
                class_hash: class_hash!("0x05ffbcfeb50d200a0677c48a129a11245a3fc519d1d98d76882d1c9a1b19c6ed"),
                selector: Some(EntryPointSelector(felt!("0x0041b033f4a31df8067c24d1e9b550a2ce75fd4a29e1147af9752174f0e6cb20"))),
            }
            .into(),
        );
        error_stack.push(
            ErrorStackSegment::StringFrame("Execution failed. Failure reason:\nError in contract (contract address: 0x06f373b346561036d98ea10fb3e60d2f459c872b1933b50b21fe6ef4fda3b75e, class hash: 0x05ffbcfeb50d200a0677c48a129a11245a3fc519d1d98d76882d1c9a1b19c6ed, selector: 0x0041b033f4a31df8067c24d1e9b550a2ce75fd4a29e1147af9752174f0e6cb20):\n0x753235365f737562204f766572666c6f77 ('u256_sub Overflow').\n".to_string()),
        );

        let revert_error = RevertError::Execution(error_stack);

        // Verify that the RevertError structure produces the correct input (unfiltered)
        let input = "Transaction execution has failed:\n0: Error in the called contract (contract address: 0x05743c833ed33a3433f1c5587dac97753ddcc84f9844e6fa2a3268e5ae35cbc3, class hash: 0x073414441639dcd11d1846f287650a00c60c416b9d3ba45d31c651672125b2c2, selector: 0x015d40a3d6ca2ac30f4031e42be28da9b056fef9bb7357ac5e85627ee876e5ad):\nError at pc=0:35988:\nCairo traceback (most recent call last):\nUnknown location (pc=0:330)\nUnknown location (pc=0:11695)\nUnknown location (pc=0:36001)\nUnknown location (pc=0:36001)\nUnknown location (pc=0:36001)\n\n1: Error in the called contract (contract address: 0x0286003f7c7bfc3f94e8f0af48b48302e7aee2fb13c23b141479ba00832ef2c6, class hash: 0x03e283b1e8bce178469acb94700999ecc7ad180420201e16eb0a81294ae8599b, selector: 0x0056878e39e16b42520b0d7936d3fd3498f86ceda4dbad50f6ff717644c95ed6):\nError at pc=0:115867:\nCairo traceback (most recent call last):\nUnknown location (pc=0:9435)\nUnknown location (pc=0:43555)\nUnknown location (pc=0:93296)\n\n2: Error in the called contract (contract address: 0x06f373b346561036d98ea10fb3e60d2f459c872b1933b50b21fe6ef4fda3b75e, class hash: 0x070cdfaea3ec997bd3a8cdedfc0ffe804a58afc3d6b5a6e5c0218ec233ceea6d, selector: 0x0041b033f4a31df8067c24d1e9b550a2ce75fd4a29e1147af9752174f0e6cb20):\nError at pc=0:32:\nCairo traceback (most recent call last):\nUnknown location (pc=0:1683)\nUnknown location (pc=0:1669)\n\n3: Error in a library call (contract address: 0x06f373b346561036d98ea10fb3e60d2f459c872b1933b50b21fe6ef4fda3b75e, class hash: 0x05ffbcfeb50d200a0677c48a129a11245a3fc519d1d98d76882d1c9a1b19c6ed, selector: 0x0041b033f4a31df8067c24d1e9b550a2ce75fd4a29e1147af9752174f0e6cb20):\nExecution failed. Failure reason:\nError in contract (contract address: 0x06f373b346561036d98ea10fb3e60d2f459c872b1933b50b21fe6ef4fda3b75e, class hash: 0x05ffbcfeb50d200a0677c48a129a11245a3fc519d1d98d76882d1c9a1b19c6ed, selector: 0x0041b033f4a31df8067c24d1e9b550a2ce75fd4a29e1147af9752174f0e6cb20):\n0x753235365f737562204f766572666c6f77 ('u256_sub Overflow').\n";
        assert_eq!(revert_error.to_string(), input);

        // Expected output: only the traceback from entry 2 (last CallContract before LibraryCall) is kept
        let expected = "Transaction execution has failed:\n0: Error in the called contract (contract address: 0x05743c833ed33a3433f1c5587dac97753ddcc84f9844e6fa2a3268e5ae35cbc3, class hash: 0x073414441639dcd11d1846f287650a00c60c416b9d3ba45d31c651672125b2c2, selector: 0x015d40a3d6ca2ac30f4031e42be28da9b056fef9bb7357ac5e85627ee876e5ad):\n1: Error in the called contract (contract address: 0x0286003f7c7bfc3f94e8f0af48b48302e7aee2fb13c23b141479ba00832ef2c6, class hash: 0x03e283b1e8bce178469acb94700999ecc7ad180420201e16eb0a81294ae8599b, selector: 0x0056878e39e16b42520b0d7936d3fd3498f86ceda4dbad50f6ff717644c95ed6):\n2: Error in the called contract (contract address: 0x06f373b346561036d98ea10fb3e60d2f459c872b1933b50b21fe6ef4fda3b75e, class hash: 0x070cdfaea3ec997bd3a8cdedfc0ffe804a58afc3d6b5a6e5c0218ec233ceea6d, selector: 0x0041b033f4a31df8067c24d1e9b550a2ce75fd4a29e1147af9752174f0e6cb20):\nError at pc=0:32:\nCairo traceback (most recent call last):\nUnknown location (pc=0:1683)\nUnknown location (pc=0:1669)\n\n3: Error in a library call (contract address: 0x06f373b346561036d98ea10fb3e60d2f459c872b1933b50b21fe6ef4fda3b75e, class hash: 0x05ffbcfeb50d200a0677c48a129a11245a3fc519d1d98d76882d1c9a1b19c6ed, selector: 0x0041b033f4a31df8067c24d1e9b550a2ce75fd4a29e1147af9752174f0e6cb20):\nExecution failed. Failure reason:\nError in contract (contract address: 0x06f373b346561036d98ea10fb3e60d2f459c872b1933b50b21fe6ef4fda3b75e, class hash: 0x05ffbcfeb50d200a0677c48a129a11245a3fc519d1d98d76882d1c9a1b19c6ed, selector: 0x0041b033f4a31df8067c24d1e9b550a2ce75fd4a29e1147af9752174f0e6cb20):\n0x753235365f737562204f766572666c6f77 ('u256_sub Overflow').\n";

        let result = revert_error.format_for_receipt().to_string();

        assert_eq!(result, expected);
    }

    #[test]
    fn test_format_for_receipt_filters_redundant_tracebacks_2() {
        use cairo_vm::types::relocatable::Relocatable;
        use starknet_api::core::EntryPointSelector;
        use starknet_api::{class_hash, contract_address, felt};

        // Build the ErrorStack structure that represents the unfiltered error
        let mut error_stack = ErrorStack {
            header: ErrorStackHeader::Execution,
            stack: vec![],
        };

        // Entry 0: CallContract with VM traceback (should be kept - next is LibraryCall)
        error_stack.push(
            EntryPointErrorFrame {
                depth: 0,
                preamble_type: PreambleType::CallContract,
                storage_address: contract_address!("0x006fb38baf7a14acc032ff556c2791b03292861581572d02296c5093fd16cafb"),
                class_hash: class_hash!("0x03530cc4759d78042f1b543bf797f5f3d647cde0388c33734cf91b7f7b9314a9"),
                selector: Some(EntryPointSelector(felt!("0x015d40a3d6ca2ac30f4031e42be28da9b056fef9bb7357ac5e85627ee876e5ad"))),
            }
            .into(),
        );
        error_stack.push(
            VmExceptionFrame {
                pc: Relocatable { segment_index: 0, offset: 12 },
                error_attr_value: None,
                traceback: Some("Cairo traceback (most recent call last):\nUnknown location (pc=0:161)\nUnknown location (pc=0:147)\n".to_string()),
            }
            .into(),
        );

        // Entry 1: LibraryCall with VM traceback (should be kept - belongs to LibraryCall)
        error_stack.push(
            EntryPointErrorFrame {
                depth: 1,
                preamble_type: PreambleType::LibraryCall,
                storage_address: contract_address!("0x006fb38baf7a14acc032ff556c2791b03292861581572d02296c5093fd16cafb"),
                class_hash: class_hash!("0x041cb0280ebadaa75f996d8d92c6f265f6d040bb3ba442e5f86a554f1765244e"),
                selector: Some(EntryPointSelector(felt!("0x015d40a3d6ca2ac30f4031e42be28da9b056fef9bb7357ac5e85627ee876e5ad"))),
            }
            .into(),
        );
        error_stack.push(
            VmExceptionFrame {
                pc: Relocatable { segment_index: 0, offset: 56 },
                error_attr_value: None,
                traceback: Some("Cairo traceback (most recent call last):\nUnknown location (pc=0:1700)\nUnknown location (pc=0:1655)\nError message: multicall 405852601487139132244494309743039711091605094719341446212637486410648343561 failed\nUnknown location (pc=0:179)\n".to_string()),
            }
            .into(),
        );

        // Entry 2: CallContract with VM traceback (should be kept - next is LibraryCall)
        error_stack.push(
            EntryPointErrorFrame {
                depth: 2,
                preamble_type: PreambleType::CallContract,
                storage_address: contract_address!("0x046e9237f5408b5f899e72125dd69bd55485a287aaf24663d3ebe00d237fc7ef"),
                class_hash: class_hash!("0x070cdfaea3ec997bd3a8cdedfc0ffe804a58afc3d6b5a6e5c0218ec233ceea6d"),
                selector: Some(EntryPointSelector(felt!("0x00e5b455a836c7a254df57ed39d023d46b641b331162c6c0b369647056655409"))),
            }
            .into(),
        );
        error_stack.push(
            VmExceptionFrame {
                pc: Relocatable { segment_index: 0, offset: 32 },
                error_attr_value: None,
                traceback: Some("Cairo traceback (most recent call last):\nUnknown location (pc=0:1683)\nUnknown location (pc=0:1669)\n".to_string()),
            }
            .into(),
        );

        // Entry 3: LibraryCall with final error
        error_stack.push(
            EntryPointErrorFrame {
                depth: 3,
                preamble_type: PreambleType::LibraryCall,
                storage_address: contract_address!("0x046e9237f5408b5f899e72125dd69bd55485a287aaf24663d3ebe00d237fc7ef"),
                class_hash: class_hash!("0x0358663e6ed9d37efd33d4661e20b2bad143e0f92076b0c91fe65f31ccf55046"),
                selector: Some(EntryPointSelector(felt!("0x00e5b455a836c7a254df57ed39d023d46b641b331162c6c0b369647056655409"))),
            }
            .into(),
        );
        error_stack.push(
            ErrorStackSegment::StringFrame("Execution failed. Failure reason:\nError in contract (contract address: 0x046e9237f5408b5f899e72125dd69bd55485a287aaf24663d3ebe00d237fc7ef, class hash: 0x0358663e6ed9d37efd33d4661e20b2bad143e0f92076b0c91fe65f31ccf55046, selector: 0x00e5b455a836c7a254df57ed39d023d46b641b331162c6c0b369647056655409):\n0x4661696c656420746f20646573657269616c697a6520706172616d202333 ('Failed to deserialize param #3').\n".to_string()),
        );

        let revert_error = RevertError::Execution(error_stack);

        // Verify that the RevertError structure produces the correct input (unfiltered)
        let input = "Transaction execution has failed:\n0: Error in the called contract (contract address: 0x006fb38baf7a14acc032ff556c2791b03292861581572d02296c5093fd16cafb, class hash: 0x03530cc4759d78042f1b543bf797f5f3d647cde0388c33734cf91b7f7b9314a9, selector: 0x015d40a3d6ca2ac30f4031e42be28da9b056fef9bb7357ac5e85627ee876e5ad):\nError at pc=0:12:\nCairo traceback (most recent call last):\nUnknown location (pc=0:161)\nUnknown location (pc=0:147)\n\n1: Error in a library call (contract address: 0x006fb38baf7a14acc032ff556c2791b03292861581572d02296c5093fd16cafb, class hash: 0x041cb0280ebadaa75f996d8d92c6f265f6d040bb3ba442e5f86a554f1765244e, selector: 0x015d40a3d6ca2ac30f4031e42be28da9b056fef9bb7357ac5e85627ee876e5ad):\nError at pc=0:56:\nCairo traceback (most recent call last):\nUnknown location (pc=0:1700)\nUnknown location (pc=0:1655)\nError message: multicall 405852601487139132244494309743039711091605094719341446212637486410648343561 failed\nUnknown location (pc=0:179)\n\n2: Error in the called contract (contract address: 0x046e9237f5408b5f899e72125dd69bd55485a287aaf24663d3ebe00d237fc7ef, class hash: 0x070cdfaea3ec997bd3a8cdedfc0ffe804a58afc3d6b5a6e5c0218ec233ceea6d, selector: 0x00e5b455a836c7a254df57ed39d023d46b641b331162c6c0b369647056655409):\nError at pc=0:32:\nCairo traceback (most recent call last):\nUnknown location (pc=0:1683)\nUnknown location (pc=0:1669)\n\n3: Error in a library call (contract address: 0x046e9237f5408b5f899e72125dd69bd55485a287aaf24663d3ebe00d237fc7ef, class hash: 0x0358663e6ed9d37efd33d4661e20b2bad143e0f92076b0c91fe65f31ccf55046, selector: 0x00e5b455a836c7a254df57ed39d023d46b641b331162c6c0b369647056655409):\nExecution failed. Failure reason:\nError in contract (contract address: 0x046e9237f5408b5f899e72125dd69bd55485a287aaf24663d3ebe00d237fc7ef, class hash: 0x0358663e6ed9d37efd33d4661e20b2bad143e0f92076b0c91fe65f31ccf55046, selector: 0x00e5b455a836c7a254df57ed39d023d46b641b331162c6c0b369647056655409):\n0x4661696c656420746f20646573657269616c697a6520706172616d202333 ('Failed to deserialize param #3').\n";
        assert_eq!(revert_error.to_string(), input);

        // Expected output: all tracebacks kept (entry 0 before LibraryCall, entry 1 belongs to LibraryCall, entry 2 before LibraryCall)
        let expected = "Transaction execution has failed:\n0: Error in the called contract (contract address: 0x006fb38baf7a14acc032ff556c2791b03292861581572d02296c5093fd16cafb, class hash: 0x03530cc4759d78042f1b543bf797f5f3d647cde0388c33734cf91b7f7b9314a9, selector: 0x015d40a3d6ca2ac30f4031e42be28da9b056fef9bb7357ac5e85627ee876e5ad):\nError at pc=0:12:\nCairo traceback (most recent call last):\nUnknown location (pc=0:161)\nUnknown location (pc=0:147)\n\n1: Error in a library call (contract address: 0x006fb38baf7a14acc032ff556c2791b03292861581572d02296c5093fd16cafb, class hash: 0x041cb0280ebadaa75f996d8d92c6f265f6d040bb3ba442e5f86a554f1765244e, selector: 0x015d40a3d6ca2ac30f4031e42be28da9b056fef9bb7357ac5e85627ee876e5ad):\nError at pc=0:56:\nCairo traceback (most recent call last):\nUnknown location (pc=0:1700)\nUnknown location (pc=0:1655)\nError message: multicall 405852601487139132244494309743039711091605094719341446212637486410648343561 failed\nUnknown location (pc=0:179)\n\n2: Error in the called contract (contract address: 0x046e9237f5408b5f899e72125dd69bd55485a287aaf24663d3ebe00d237fc7ef, class hash: 0x070cdfaea3ec997bd3a8cdedfc0ffe804a58afc3d6b5a6e5c0218ec233ceea6d, selector: 0x00e5b455a836c7a254df57ed39d023d46b641b331162c6c0b369647056655409):\nError at pc=0:32:\nCairo traceback (most recent call last):\nUnknown location (pc=0:1683)\nUnknown location (pc=0:1669)\n\n3: Error in a library call (contract address: 0x046e9237f5408b5f899e72125dd69bd55485a287aaf24663d3ebe00d237fc7ef, class hash: 0x0358663e6ed9d37efd33d4661e20b2bad143e0f92076b0c91fe65f31ccf55046, selector: 0x00e5b455a836c7a254df57ed39d023d46b641b331162c6c0b369647056655409):\nExecution failed. Failure reason:\nError in contract (contract address: 0x046e9237f5408b5f899e72125dd69bd55485a287aaf24663d3ebe00d237fc7ef, class hash: 0x0358663e6ed9d37efd33d4661e20b2bad143e0f92076b0c91fe65f31ccf55046, selector: 0x00e5b455a836c7a254df57ed39d023d46b641b331162c6c0b369647056655409):\n0x4661696c656420746f20646573657269616c697a6520706172616d202333 ('Failed to deserialize param #3').\n";

        let result = revert_error.format_for_receipt().to_string();

        assert_eq!(result, expected);
    }
}
