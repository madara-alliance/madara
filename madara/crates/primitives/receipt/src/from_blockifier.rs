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

/// Formats the revert error string by removing redundant VM tracebacks.
///
/// The blockifier generates verbose error messages with VM tracebacks at every level
/// of the call stack. This function filters them to show the traceback only once,
/// positioned after the last regular contract call (CallContract) entry point frame
/// and before any library call (LibraryCall) frames or the final error.
///
/// This makes error messages more concise while preserving the most relevant debugging information.
fn format_revert_error(error_string: &str) -> String {
    // Check if the original string ends with a newline
    let ends_with_newline = error_string.ends_with('\n');

    let lines: Vec<&str> = error_string.lines().collect();
    let mut output_lines = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];

        // Check if this is the start of a VM exception frame
        // VM exception frames start with "Error at pc=..."
        if line.trim().starts_with("Error at pc=") {
            // Look ahead to see if there's another "Error in the called contract" entry
            // or "Error in a library call" coming up
            let mut has_nested_call_contract = false;
            let mut j = i + 1;

            // Scan forward to find the next entry point frame
            while j < lines.len() {
                let next_line = lines[j].trim();

                // Found the next entry point - check if it's a CallContract or LibraryCall
                if next_line.starts_with("Error in the called contract") {
                    has_nested_call_contract = true;
                    break;
                } else if next_line.starts_with("Error in a library call")
                    || next_line.starts_with("Execution failed")
                    || next_line.starts_with("Entry point") {
                    // Next entry is a library call or final error - don't skip this traceback
                    has_nested_call_contract = false;
                    break;
                }

                // Keep scanning through the traceback lines
                if next_line.starts_with("Unknown location")
                    || next_line.starts_with("Cairo traceback")
                    || next_line.is_empty() {
                    j += 1;
                } else {
                    // Reached a numbered entry like "1:" - check if it contains the patterns
                    if let Some(colon_pos) = next_line.find(':') {
                        let after_colon = &next_line[colon_pos + 1..].trim();
                        if after_colon.starts_with("Error in the called contract") {
                            has_nested_call_contract = true;
                            break;
                        } else if after_colon.starts_with("Error in a library call") {
                            has_nested_call_contract = false;
                            break;
                        }
                    }
                    j += 1;
                }
            }

            // Skip the VM traceback if there's a nested CallContract
            if has_nested_call_contract {
                // Skip this "Error at pc=..." line and all traceback lines until the next entry
                while i < lines.len() {
                    let current_line = lines[i].trim();
                    i += 1;

                    // Stop when we hit the next numbered entry or end
                    if i < lines.len() {
                        let next_line = lines[i].trim();
                        if let Some(first_char) = next_line.chars().next() {
                            if first_char.is_ascii_digit() && next_line.contains(':') {
                                break;
                            }
                        }
                    }

                    // Also stop at the end or at lines that look like new entries
                    if current_line.is_empty() && i < lines.len() {
                        let peek = lines[i].trim();
                        if let Some(first_char) = peek.chars().next() {
                            if first_char.is_ascii_digit() || peek.starts_with("Execution failed") {
                                break;
                            }
                        }
                    }
                }
            } else {
                // Keep this traceback - it's before a library call or final error
                output_lines.push(line);
                i += 1;
            }
        } else {
            // Not a VM traceback line - keep it
            output_lines.push(line);
            i += 1;
        }
    }

    let mut result = output_lines.join("\n");

    // Preserve the trailing newline if the original had one
    if ends_with_newline {
        result.push('\n');
    }

    result
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
        let formatted_reason = format_revert_error(&reason.to_string());
        ExecutionResult::Reverted { reason: formatted_reason }
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
    fn test_format_revert_error_filters_redundant_tracebacks() {
        // Real example from blockifier with VM tracebacks at every CallContract level
        let input = "Transaction execution has failed:\n0: Error in the called contract (contract address: 0x05743c833ed33a3433f1c5587dac97753ddcc84f9844e6fa2a3268e5ae35cbc3, class hash: 0x073414441639dcd11d1846f287650a00c60c416b9d3ba45d31c651672125b2c2, selector: 0x015d40a3d6ca2ac30f4031e42be28da9b056fef9bb7357ac5e85627ee876e5ad):\nError at pc=0:35988:\nCairo traceback (most recent call last):\nUnknown location (pc=0:330)\nUnknown location (pc=0:11695)\nUnknown location (pc=0:36001)\nUnknown location (pc=0:36001)\nUnknown location (pc=0:36001)\n\n1: Error in the called contract (contract address: 0x0286003f7c7bfc3f94e8f0af48b48302e7aee2fb13c23b141479ba00832ef2c6, class hash: 0x03e283b1e8bce178469acb94700999ecc7ad180420201e16eb0a81294ae8599b, selector: 0x0056878e39e16b42520b0d7936d3fd3498f86ceda4dbad50f6ff717644c95ed6):\nError at pc=0:115867:\nCairo traceback (most recent call last):\nUnknown location (pc=0:9435)\nUnknown location (pc=0:43555)\nUnknown location (pc=0:93296)\n\n2: Error in the called contract (contract address: 0x06f373b346561036d98ea10fb3e60d2f459c872b1933b50b21fe6ef4fda3b75e, class hash: 0x070cdfaea3ec997bd3a8cdedfc0ffe804a58afc3d6b5a6e5c0218ec233ceea6d, selector: 0x0041b033f4a31df8067c24d1e9b550a2ce75fd4a29e1147af9752174f0e6cb20):\nError at pc=0:32:\nCairo traceback (most recent call last):\nUnknown location (pc=0:1683)\nUnknown location (pc=0:1669)\n\n3: Error in a library call (contract address: 0x06f373b346561036d98ea10fb3e60d2f459c872b1933b50b21fe6ef4fda3b75e, class hash: 0x05ffbcfeb50d200a0677c48a129a11245a3fc519d1d98d76882d1c9a1b19c6ed, selector: 0x0041b033f4a31df8067c24d1e9b550a2ce75fd4a29e1147af9752174f0e6cb20):\nExecution failed. Failure reason:\nError in contract (contract address: 0x06f373b346561036d98ea10fb3e60d2f459c872b1933b50b21fe6ef4fda3b75e, class hash: 0x05ffbcfeb50d200a0677c48a129a11245a3fc519d1d98d76882d1c9a1b19c6ed, selector: 0x0041b033f4a31df8067c24d1e9b550a2ce75fd4a29e1147af9752174f0e6cb20):\n0x753235365f737562204f766572666c6f77 ('u256_sub Overflow').\n";

        // Expected output: only the traceback from entry 2 (last CallContract before LibraryCall) is kept
        let expected = "Transaction execution has failed:\n0: Error in the called contract (contract address: 0x05743c833ed33a3433f1c5587dac97753ddcc84f9844e6fa2a3268e5ae35cbc3, class hash: 0x073414441639dcd11d1846f287650a00c60c416b9d3ba45d31c651672125b2c2, selector: 0x015d40a3d6ca2ac30f4031e42be28da9b056fef9bb7357ac5e85627ee876e5ad):\n1: Error in the called contract (contract address: 0x0286003f7c7bfc3f94e8f0af48b48302e7aee2fb13c23b141479ba00832ef2c6, class hash: 0x03e283b1e8bce178469acb94700999ecc7ad180420201e16eb0a81294ae8599b, selector: 0x0056878e39e16b42520b0d7936d3fd3498f86ceda4dbad50f6ff717644c95ed6):\n2: Error in the called contract (contract address: 0x06f373b346561036d98ea10fb3e60d2f459c872b1933b50b21fe6ef4fda3b75e, class hash: 0x070cdfaea3ec997bd3a8cdedfc0ffe804a58afc3d6b5a6e5c0218ec233ceea6d, selector: 0x0041b033f4a31df8067c24d1e9b550a2ce75fd4a29e1147af9752174f0e6cb20):\nError at pc=0:32:\nCairo traceback (most recent call last):\nUnknown location (pc=0:1683)\nUnknown location (pc=0:1669)\n\n3: Error in a library call (contract address: 0x06f373b346561036d98ea10fb3e60d2f459c872b1933b50b21fe6ef4fda3b75e, class hash: 0x05ffbcfeb50d200a0677c48a129a11245a3fc519d1d98d76882d1c9a1b19c6ed, selector: 0x0041b033f4a31df8067c24d1e9b550a2ce75fd4a29e1147af9752174f0e6cb20):\nExecution failed. Failure reason:\nError in contract (contract address: 0x06f373b346561036d98ea10fb3e60d2f459c872b1933b50b21fe6ef4fda3b75e, class hash: 0x05ffbcfeb50d200a0677c48a129a11245a3fc519d1d98d76882d1c9a1b19c6ed, selector: 0x0041b033f4a31df8067c24d1e9b550a2ce75fd4a29e1147af9752174f0e6cb20):\n0x753235365f737562204f766572666c6f77 ('u256_sub Overflow').\n";

        let result = format_revert_error(input);

        assert_eq!(result, expected);
    }

    #[test]
    fn test_format_revert_error_preserves_trailing_newline() {
        let input_with_newline = "Error at pc=0:123:\nCairo traceback\n";
        let input_without_newline = "Error at pc=0:123:\nCairo traceback";

        let result_with = format_revert_error(input_with_newline);
        let result_without = format_revert_error(input_without_newline);

        assert!(result_with.ends_with('\n'));
        assert!(!result_without.ends_with('\n'));
    }

    #[test]
    fn test_format_revert_error_keeps_traceback_before_library_call() {
        let input = "0: Error in the called contract:\nError at pc=0:100:\nCairo traceback\n\n1: Error in a library call:\nExecution failed\n";
        let result = format_revert_error(input);

        // Should keep the traceback because it's before a library call
        assert!(result.contains("Error at pc=0:100:"));
        assert!(result.contains("Cairo traceback"));
    }

    #[test]
    fn test_format_revert_error_skips_traceback_with_nested_call_contract() {
        let input = "0: Error in the called contract:\nError at pc=0:100:\nCairo traceback\n\n1: Error in the called contract:\nError at pc=0:200:\nAnother traceback\n";
        let result = format_revert_error(input);

        // Should skip the first traceback because there's a nested CallContract
        assert!(!result.contains("Error at pc=0:100:"));
        assert!(!result.contains("Cairo traceback"));
        // Should keep the second one
        assert!(result.contains("Error at pc=0:200:"));
        assert!(result.contains("Another traceback"));
    }

    #[test]
    fn test_format_revert_error_no_tracebacks() {
        let input = "Transaction execution has failed:\nExecution failed. Failure reason: 'Some error'\n";
        let result = format_revert_error(input);

        // Should preserve the error message as-is when there are no tracebacks
        assert_eq!(result, input);
    }
}
