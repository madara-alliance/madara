//! Builds the spec `CONTRACT_EXECUTION_ERROR` object (RPC spec >= v0.8) from blockifier error
//! stacks: a chain of `{ contract_address, class_hash, selector, error }` call frames whose
//! innermost `error` is the failure-reason string.
//!
//! The flattening and folding semantics match pathfinder (`crates/executor/src/error_stack.rs` +
//! `error_stack_frames_to_json`) and juno (`vm/rust/src/error/stack.rs`): entrypoint frames and
//! Cairo 1 revert frames become call frames, the *last* string frame (VM traceback, panic data,
//! or plain message) becomes the leaf, and call frames are folded around it outermost-first.

use blockifier::execution::stack_trace::{
    gen_tx_execution_error_trace, Cairo1RevertSummary, ErrorStack, ErrorStackSegment,
};
use blockifier::transaction::errors::TransactionExecutionError;
use blockifier::transaction::objects::RevertError;
use serde_json::json;
use starknet_types_core::felt::Felt;

enum Frame {
    Call { contract_address: Felt, class_hash: Felt, selector: Option<Felt> },
    Str(String),
}

fn push_cairo1_revert_summary(summary: &Cairo1RevertSummary, frames: &mut Vec<Frame>) {
    frames.extend(summary.stack.iter().map(|frame| Frame::Call {
        contract_address: *frame.contract_address.0.key(),
        class_hash: frame.class_hash.unwrap_or_default().0,
        selector: Some(frame.selector.0),
    }));
    frames.push(Frame::Str(starknet_api::execution_utils::format_panic_data(&summary.last_retdata.0)));
}

fn flatten_error_stack(stack: &ErrorStack) -> Vec<Frame> {
    let mut frames = Vec::new();
    for segment in &stack.stack {
        match segment {
            ErrorStackSegment::EntryPoint(entry_point) => frames.push(Frame::Call {
                contract_address: *entry_point.storage_address.0.key(),
                class_hash: entry_point.class_hash.0,
                selector: entry_point.selector.map(|s| s.0),
            }),
            ErrorStackSegment::Cairo1RevertSummary(summary) => push_cairo1_revert_summary(summary, &mut frames),
            ErrorStackSegment::Vm(vm_exception) => frames.push(Frame::Str(String::from(vm_exception))),
            ErrorStackSegment::StringFrame(string) => frames.push(Frame::Str(string.clone())),
        }
    }
    frames
}

fn frames_to_json(frames: Vec<Frame>) -> serde_json::Value {
    let leaf = frames
        .iter()
        .rev()
        .find_map(|frame| match frame {
            Frame::Str(string) => Some(string.clone()),
            _ => None,
        })
        .unwrap_or_else(|| "Unknown error, no string frame available.".to_string());

    frames
        .into_iter()
        .filter_map(|frame| match frame {
            Frame::Call { contract_address, class_hash, selector } => Some((contract_address, class_hash, selector)),
            _ => None,
        })
        .rev()
        .fold(json!(leaf), |child, (contract_address, class_hash, selector)| {
            let mut frame = json!({
                "contract_address": contract_address,
                "class_hash": class_hash,
                "error": child,
            });
            // Constructor frames have no selector: omit the key instead of serializing null.
            if let Some(selector) = selector {
                frame["selector"] = json!(selector);
            }
            frame
        })
}

/// The `CONTRACT_EXECUTION_ERROR` for a hard execution error (blockifier `Err`).
pub fn contract_execution_error(error: &TransactionExecutionError) -> serde_json::Value {
    frames_to_json(flatten_error_stack(&gen_tx_execution_error_trace(error)))
}

/// The `CONTRACT_EXECUTION_ERROR` for a reverted execution (blockifier `Ok` with `revert_error`).
pub fn contract_execution_error_from_revert(error: &RevertError) -> serde_json::Value {
    match error {
        RevertError::Execution(error_stack) => frames_to_json(flatten_error_stack(error_stack)),
        RevertError::PostExecution(fee_check_error) => json!(fee_check_error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use blockifier::execution::call_info::Retdata;
    use blockifier::execution::stack_trace::{Cairo1RevertFrame, EntryPointErrorFrame, ErrorStackHeader, PreambleType};
    use starknet_api::core::EntryPointSelector;
    use starknet_api::{class_hash, contract_address, felt};

    #[test]
    fn nested_call_frames_fold_around_last_string_frame() {
        let mut stack = ErrorStack { header: ErrorStackHeader::Execution, stack: Vec::new() };
        stack.push(ErrorStackSegment::EntryPoint(EntryPointErrorFrame {
            depth: 0,
            preamble_type: PreambleType::CallContract,
            storage_address: contract_address!("0xa1"),
            class_hash: class_hash!("0xb1"),
            selector: Some(EntryPointSelector(felt!("0xc1"))),
        }));
        // Intermediate string frames (e.g. VM tracebacks) are dropped: only the last one is the
        // leaf, matching pathfinder and juno.
        stack.push(ErrorStackSegment::StringFrame("intermediate vm traceback".to_string()));
        stack.push(ErrorStackSegment::EntryPoint(EntryPointErrorFrame {
            depth: 1,
            preamble_type: PreambleType::CallContract,
            storage_address: contract_address!("0xa2"),
            class_hash: class_hash!("0xb2"),
            selector: Some(EntryPointSelector(felt!("0xc2"))),
        }));
        stack.push(ErrorStackSegment::StringFrame("the actual failure".to_string()));

        let json = frames_to_json(flatten_error_stack(&stack));

        assert_eq!(
            json,
            json!({
                "contract_address": "0xa1",
                "class_hash": "0xb1",
                "selector": "0xc1",
                "error": {
                    "contract_address": "0xa2",
                    "class_hash": "0xb2",
                    "selector": "0xc2",
                    "error": "the actual failure",
                }
            })
        );
    }

    #[test]
    fn cairo1_revert_summary_formats_panic_data_as_leaf() {
        let stack = ErrorStack {
            header: ErrorStackHeader::Execution,
            stack: vec![ErrorStackSegment::Cairo1RevertSummary(Cairo1RevertSummary {
                header: blockifier::execution::stack_trace::Cairo1RevertHeader::Execution,
                stack: vec![Cairo1RevertFrame {
                    contract_address: contract_address!("0xa1"),
                    class_hash: Some(class_hash!("0xb1")),
                    selector: EntryPointSelector(felt!("0xc1")),
                }],
                // 'ENTRYPOINT_NOT_FOUND'
                last_retdata: Retdata(vec![felt!("0x454e545259504f494e545f4e4f545f464f554e44")]),
            })],
        };

        let json = frames_to_json(flatten_error_stack(&stack));

        assert_eq!(
            json,
            json!({
                "contract_address": "0xa1",
                "class_hash": "0xb1",
                "selector": "0xc1",
                "error": "0x454e545259504f494e545f4e4f545f464f554e44 ('ENTRYPOINT_NOT_FOUND')",
            })
        );
    }

    #[test]
    fn no_string_frame_yields_fallback_leaf() {
        let stack = ErrorStack { header: ErrorStackHeader::Execution, stack: Vec::new() };
        assert_eq!(frames_to_json(flatten_error_stack(&stack)), json!("Unknown error, no string frame available."));
    }

    /// Constructor frames have no selector: the key is omitted, not serialized as null.
    #[test]
    fn constructor_frame_omits_selector() {
        let stack = ErrorStack {
            header: ErrorStackHeader::Constructor,
            stack: vec![
                ErrorStackSegment::EntryPoint(EntryPointErrorFrame {
                    depth: 0,
                    preamble_type: PreambleType::Constructor,
                    storage_address: contract_address!("0xa1"),
                    class_hash: class_hash!("0xb1"),
                    selector: None,
                }),
                ErrorStackSegment::StringFrame("constructor failed".to_string()),
            ],
        };

        let json = frames_to_json(flatten_error_stack(&stack));

        assert_eq!(
            json,
            json!({
                "contract_address": "0xa1",
                "class_hash": "0xb1",
                "error": "constructor failed",
            })
        );
    }
}
