//! RevertError formatting utilities for filtering redundant VM tracebacks.
//!
//! This module provides extension traits and utilities for formatting blockifier's
//! `RevertError` type to produce cleaner, more readable error messages by filtering
//! redundant VM tracebacks that appear at every level of the call stack.

use blockifier::execution::stack_trace::{ErrorStack, ErrorStackSegment, PreambleType, VmExceptionFrame};
use blockifier::transaction::objects::RevertError;

/// Extension trait for `RevertError` to provide proper formatting with filtered VM tracebacks.
///
/// This trait implements the `format_for_receipt()` method.
/// It filters redundant VM tracebacks from error stacks to make error messages more readable.
pub trait RevertErrorExt {
    /// Returns a new RevertError with filtered VM tracebacks (consuming version).
    ///
    /// This method leverages the typed structure of `RevertError`. For execution errors,
    /// it filters redundant VM tracebacks from the error stack, keeping only the most
    /// relevant ones. For post-execution errors, it returns the original error moved.
    ///
    /// Note: This method consumes self to avoid cloning non-Clone types.
    ///
    /// # Returns
    /// A new `RevertError` with filtered error information.
    fn format_for_receipt(self) -> RevertError;

    /// Returns a formatted string representation with filtered VM tracebacks (non-consuming version).
    ///
    /// This method is used when you only have a reference and need the formatted string output.
    /// It directly formats to string without needing to clone the RevertError.
    ///
    /// # Returns
    /// A formatted string with filtered error information.
    fn format_for_receipt_string(&self) -> String;
}

impl RevertErrorExt for RevertError {
    fn format_for_receipt(self) -> RevertError {
        match self {
            RevertError::Execution(error_stack) => {
                // Create a new ErrorStack with filtered segments
                let new_stack = filter_redundant_vm_tracebacks(error_stack);
                RevertError::Execution(new_stack)
            }
            RevertError::PostExecution(fee_error) => RevertError::PostExecution(fee_error),
        }
    }

    fn format_for_receipt_string(&self) -> String {
        match self {
            RevertError::Execution(error_stack) => {
                // Create a filtered version for display purposes only
                let filtered = filter_redundant_vm_tracebacks_ref(error_stack);
                filtered.to_string()
            }
            RevertError::PostExecution(fee_error) => fee_error.to_string(),
        }
    }
}

/// Determines whether a VM traceback at the given index should be kept.
///
/// Rules:
/// - Always keep VM tracebacks that belong to LibraryCall entries
/// - For CallContract entries:
///   - Keep if there's no EntryPoint after (it's the last one before the error)
///   - Keep if there's a LibraryCall before this CallContract (we're inside a library call context)
///   - Keep if the next EntryPoint is a LibraryCall (context before entering library)
///   - Keep if the next CallContract is the last one before error (preserve call chain context)
///   - Remove if followed by another CallContract that will eventually lead to a LibraryCall
/// - If no owning entry point is found, keep the traceback (safety default)
fn should_keep_vm_traceback(error_stack: &ErrorStack, vm_index: usize) -> bool {
    let owning_entry = find_parent_entry_point(error_stack, vm_index);

    match owning_entry {
        Some(entry_point) => {
            // Always keep VM tracebacks for LibraryCall entries
            if entry_point.preamble_type == PreambleType::LibraryCall {
                true
            } else {
                // For CallContract entries, check what comes next
                // Keep if:
                // 1. There's no EntryPoint after this (last in chain), OR
                // 2. There's a LibraryCall before (inside library call context), OR
                // 3. The next EntryPoint is a LibraryCall without its own VM traceback, OR
                // 4. The next CallContract is the last one before error (preserve chain context for CallContract-only chains)
                !has_any_entry_point_after(error_stack, vm_index)
                    || has_library_call_before(error_stack, vm_index)
                    || is_next_entry_library_call_without_vm(error_stack, vm_index)
                    || is_next_call_contract_last(error_stack, vm_index)
            }
        }
        // If we can't find an owning entry, keep the traceback
        None => true,
    }
}

/// Filters redundant VM tracebacks from the error stack.
///
/// The blockifier generates VM tracebacks at every level of the call stack.
/// This function filters them to show the traceback only once, positioned after
/// the last regular contract call (CallContract) entry point frame and before
/// any library call (LibraryCall) frames or the final error.
fn filter_redundant_vm_tracebacks(error_stack: ErrorStack) -> ErrorStack {
    let len = error_stack.stack.len();

    // Create a list of indices to keep
    let mut indices_to_keep = Vec::new();

    for i in 0..len {
        let segment = &error_stack.stack[i];

        match segment {
            ErrorStackSegment::Vm(_) => {
                if should_keep_vm_traceback(&error_stack, i) {
                    indices_to_keep.push(i);
                }
            }
            // Always keep non-VM segments
            _ => indices_to_keep.push(i),
        }
    }

    // Move the segments we want to keep into the new stack
    let mut stack_vec = error_stack.stack;
    for (new_idx, original_idx) in indices_to_keep.iter().enumerate() {
        // For indices we're keeping, move them to the front of the vec
        if new_idx != *original_idx {
            stack_vec.swap(new_idx, *original_idx);
        }
    }

    // Truncate to only keep the elements we want
    stack_vec.truncate(indices_to_keep.len());

    ErrorStack { header: error_stack.header, stack: stack_vec }
}

/// Filters redundant VM tracebacks from the error stack (reference version for display).
///
/// This version works with references and constructs a new ErrorStack for display purposes.
/// Used when we can't move out of the original error stack.
fn filter_redundant_vm_tracebacks_ref(error_stack: &ErrorStack) -> ErrorStack {
    let mut new_stack = ErrorStack { header: error_stack.header.clone(), stack: Vec::new() };

    let len = error_stack.stack.len();

    for i in 0..len {
        let segment = &error_stack.stack[i];

        match segment {
            ErrorStackSegment::Vm(vm_frame) => {
                if should_keep_vm_traceback(error_stack, i) {
                    // Manually reconstruct the VmExceptionFrame
                    new_stack.push(ErrorStackSegment::Vm(VmExceptionFrame {
                        pc: vm_frame.pc,
                        error_attr_value: vm_frame.error_attr_value.clone(),
                        traceback: vm_frame.traceback.clone(),
                    }));
                }
            }
            ErrorStackSegment::EntryPoint(entry_point) => {
                // Manually reconstruct the EntryPointErrorFrame
                new_stack.push(ErrorStackSegment::EntryPoint(
                    blockifier::execution::stack_trace::EntryPointErrorFrame {
                        depth: entry_point.depth,
                        preamble_type: entry_point.preamble_type.clone(),
                        storage_address: entry_point.storage_address,
                        class_hash: entry_point.class_hash,
                        selector: entry_point.selector,
                    },
                ));
            }
            ErrorStackSegment::Cairo1RevertSummary(summary) => {
                new_stack.push(ErrorStackSegment::Cairo1RevertSummary(summary.clone()));
            }
            ErrorStackSegment::StringFrame(s) => {
                new_stack.push(ErrorStackSegment::StringFrame(s.clone()));
            }
        }
    }

    new_stack
}

/// Finds the EntryPoint that owns the VM traceback at the given index.
/// Looks backward from the current index to find the most recent EntryPoint.
fn find_parent_entry_point(
    error_stack: &ErrorStack,
    vm_index: usize,
) -> Option<&blockifier::execution::stack_trace::EntryPointErrorFrame> {
    for i in (0..vm_index).rev() {
        if let ErrorStackSegment::EntryPoint(entry_point) = &error_stack.stack[i] {
            return Some(entry_point);
        }
    }
    None
}

/// Checks if there's any EntryPoint (CallContract or LibraryCall) after the given VM traceback index.
/// Returns true if there's any EntryPoint after, false if this is the last one before the error.
fn has_any_entry_point_after(error_stack: &ErrorStack, vm_index: usize) -> bool {
    // Scan forward to find any EntryPoint
    for segment in error_stack.stack.iter().skip(vm_index + 1) {
        if matches!(segment, ErrorStackSegment::EntryPoint(_)) {
            return true;
        }
    }
    // No EntryPoint found after this index
    false
}

/// Checks if the next EntryPoint after this VM traceback is a LibraryCall
/// and this CallContract should keep its traceback.
///
/// Returns true only if:
/// 1. The next EntryPoint is a LibraryCall
/// 2. AND one of the following:
///    a. The LibraryCall does not have its own VM traceback, OR
///    b. This CallContract is not preceded by another CallContract (it's the first CC before LC)
///
/// This ensures we keep the CallContract's traceback when it's the first in a chain
/// before a LibraryCall, or when the LibraryCall doesn't provide its own traceback.
fn is_next_entry_library_call_without_vm(error_stack: &ErrorStack, vm_index: usize) -> bool {
    let mut found_library_call = false;
    let mut library_call_index = 0;

    // Find the next EntryPoint
    for (i, segment) in error_stack.stack.iter().enumerate().skip(vm_index + 1) {
        if let ErrorStackSegment::EntryPoint(entry_point) = segment {
            if entry_point.preamble_type == PreambleType::LibraryCall {
                found_library_call = true;
                library_call_index = i;
            }
            break;
        }
    }

    if !found_library_call {
        return false;
    }

    // Check if the LibraryCall has a VM traceback immediately after it
    let library_call_has_vm = if library_call_index + 1 < error_stack.stack.len() {
        matches!(&error_stack.stack[library_call_index + 1], ErrorStackSegment::Vm(_))
    } else {
        false
    };

    // If LibraryCall doesn't have its own VM traceback, keep the CallContract's
    if !library_call_has_vm {
        return true;
    }

    // LibraryCall has its own VM traceback.
    // Only keep CallContract's traceback if this is the first CallContract
    // (not preceded by another CallContract)
    let owning_entry_index = find_parent_entry_point_index(error_stack, vm_index);
    if let Some(idx) = owning_entry_index {
        // Check if there's a CallContract before this one
        for i in (0..idx).rev() {
            if let ErrorStackSegment::EntryPoint(prev_entry) = &error_stack.stack[i] {
                // If previous is LibraryCall, this is the first CC in a new chain - keep it
                // If previous is CallContract, this CC is redundant - don't keep it
                return prev_entry.preamble_type == PreambleType::LibraryCall;
            }
        }
    }

    // No previous EntryPoint, so this is the first - keep it
    true
}

/// Finds the index of the EntryPoint that owns the VM traceback at the given index.
fn find_parent_entry_point_index(error_stack: &ErrorStack, vm_index: usize) -> Option<usize> {
    for i in (0..vm_index).rev() {
        if matches!(&error_stack.stack[i], ErrorStackSegment::EntryPoint(_)) {
            return Some(i);
        }
    }
    None
}

/// Checks if the next CallContract after this VM traceback is the last one before the error.
/// Returns true only if:
/// 1. The next EntryPoint is a CallContract
/// 2. There are no more EntryPoints after it (just error messages)
///
/// This specifically handles the case where CallContract chain ends in error without LibraryCall.
fn is_next_call_contract_last(error_stack: &ErrorStack, vm_index: usize) -> bool {
    let mut found_next_entry = false;

    // Scan forward to find EntryPoints
    for segment in error_stack.stack.iter().skip(vm_index + 1) {
        if let ErrorStackSegment::EntryPoint(entry_point) = segment {
            if !found_next_entry {
                // This is the next entry point
                if entry_point.preamble_type != PreambleType::CallContract {
                    // Next entry is not a CallContract, so doesn't apply
                    return false;
                }
                found_next_entry = true;
            } else {
                // Found a second entry point after the next CallContract
                // If there's another EntryPoint (CallContract or LibraryCall), return false
                // We only want to keep both when the chain ends without more EntryPoints
                return false;
            }
        }
    }

    // If we found the next CallContract and no more EntryPoints after it, it's the last
    found_next_entry
}

/// Checks if there's a LibraryCall entry point before the given VM traceback index.
/// This is used to determine if we're inside a library call context, in which case
/// we should keep the traceback for CallContract entries even if they're followed by
/// more CallContracts.
fn has_library_call_before(error_stack: &ErrorStack, vm_index: usize) -> bool {
    for i in (0..vm_index).rev() {
        if let ErrorStackSegment::EntryPoint(entry_point) = &error_stack.stack[i] {
            if entry_point.preamble_type == PreambleType::LibraryCall {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use blockifier::execution::stack_trace::{EntryPointErrorFrame, ErrorStackHeader};
    use cairo_vm::types::relocatable::Relocatable;
    use starknet_api::core::EntryPointSelector;

    // Helper macro to create test addresses - works without the testing feature
    macro_rules! test_contract_address {
        ($addr:expr) => {{
            use starknet_api::core::ContractAddress;
            use starknet_api::core::PatriciaKey;
            use starknet_types_core::felt::Felt;
            ContractAddress(PatriciaKey::try_from(Felt::from_hex($addr).unwrap()).unwrap())
        }};
    }

    macro_rules! test_class_hash {
        ($hash:expr) => {{
            use starknet_api::core::ClassHash;
            use starknet_types_core::felt::Felt;
            ClassHash(Felt::from_hex($hash).unwrap())
        }};
    }

    macro_rules! test_felt {
        ($val:expr) => {{
            use starknet_types_core::felt::Felt;
            Felt::from_hex($val).unwrap()
        }};
    }

    #[test]
    fn test_revert_error_filters_call_contract_chain_before_library_call_without_vm() {
        // Test case: CC(0) -> CC(1) -> CC(2) -> LC(3, no VM, only StringFrame)
        // Only the last CallContract (entry 2) before the LibraryCall should keep its traceback
        // because the LibraryCall doesn't have its own VM traceback.
        let mut error_stack = ErrorStack { header: ErrorStackHeader::Execution, stack: vec![] };

        // Entry 0: CallContract with VM traceback (should be removed)
        error_stack.push(
            EntryPointErrorFrame {
                depth: 0,
                preamble_type: PreambleType::CallContract,
                storage_address: test_contract_address!(
                    "0x05743c833ed33a3433f1c5587dac97753ddcc84f9844e6fa2a3268e5ae35cbc3"
                ),
                class_hash: test_class_hash!("0x073414441639dcd11d1846f287650a00c60c416b9d3ba45d31c651672125b2c2"),
                selector: Some(EntryPointSelector(test_felt!(
                    "0x015d40a3d6ca2ac30f4031e42be28da9b056fef9bb7357ac5e85627ee876e5ad"
                ))),
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
                storage_address: test_contract_address!(
                    "0x0286003f7c7bfc3f94e8f0af48b48302e7aee2fb13c23b141479ba00832ef2c6"
                ),
                class_hash: test_class_hash!("0x03e283b1e8bce178469acb94700999ecc7ad180420201e16eb0a81294ae8599b"),
                selector: Some(EntryPointSelector(test_felt!(
                    "0x0056878e39e16b42520b0d7936d3fd3498f86ceda4dbad50f6ff717644c95ed6"
                ))),
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
                storage_address: test_contract_address!(
                    "0x06f373b346561036d98ea10fb3e60d2f459c872b1933b50b21fe6ef4fda3b75e"
                ),
                class_hash: test_class_hash!("0x070cdfaea3ec997bd3a8cdedfc0ffe804a58afc3d6b5a6e5c0218ec233ceea6d"),
                selector: Some(EntryPointSelector(test_felt!(
                    "0x0041b033f4a31df8067c24d1e9b550a2ce75fd4a29e1147af9752174f0e6cb20"
                ))),
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
                storage_address: test_contract_address!(
                    "0x06f373b346561036d98ea10fb3e60d2f459c872b1933b50b21fe6ef4fda3b75e"
                ),
                class_hash: test_class_hash!("0x05ffbcfeb50d200a0677c48a129a11245a3fc519d1d98d76882d1c9a1b19c6ed"),
                selector: Some(EntryPointSelector(test_felt!(
                    "0x0041b033f4a31df8067c24d1e9b550a2ce75fd4a29e1147af9752174f0e6cb20"
                ))),
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
    fn test_revert_error_keeps_tracebacks_for_alternating_call_contract_library_call_pattern() {
        // Test case: CC(0) -> LC(1, with VM) -> CC(2) -> LC(3, no VM, only StringFrame)
        // All tracebacks should be kept because:
        // - Entry 0: First CC before LC with VM (kept as first in chain)
        // - Entry 1: LibraryCall always keeps its traceback
        // - Entry 2: CC after LC context, before LC without VM
        let mut error_stack = ErrorStack { header: ErrorStackHeader::Execution, stack: vec![] };

        // Entry 0: CallContract with VM traceback (should be kept - next is LibraryCall)
        error_stack.push(
            EntryPointErrorFrame {
                depth: 0,
                preamble_type: PreambleType::CallContract,
                storage_address: test_contract_address!(
                    "0x006fb38baf7a14acc032ff556c2791b03292861581572d02296c5093fd16cafb"
                ),
                class_hash: test_class_hash!("0x03530cc4759d78042f1b543bf797f5f3d647cde0388c33734cf91b7f7b9314a9"),
                selector: Some(EntryPointSelector(test_felt!(
                    "0x015d40a3d6ca2ac30f4031e42be28da9b056fef9bb7357ac5e85627ee876e5ad"
                ))),
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
                storage_address: test_contract_address!(
                    "0x006fb38baf7a14acc032ff556c2791b03292861581572d02296c5093fd16cafb"
                ),
                class_hash: test_class_hash!("0x041cb0280ebadaa75f996d8d92c6f265f6d040bb3ba442e5f86a554f1765244e"),
                selector: Some(EntryPointSelector(test_felt!(
                    "0x015d40a3d6ca2ac30f4031e42be28da9b056fef9bb7357ac5e85627ee876e5ad"
                ))),
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
                storage_address: test_contract_address!(
                    "0x046e9237f5408b5f899e72125dd69bd55485a287aaf24663d3ebe00d237fc7ef"
                ),
                class_hash: test_class_hash!("0x070cdfaea3ec997bd3a8cdedfc0ffe804a58afc3d6b5a6e5c0218ec233ceea6d"),
                selector: Some(EntryPointSelector(test_felt!(
                    "0x00e5b455a836c7a254df57ed39d023d46b641b331162c6c0b369647056655409"
                ))),
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
                storage_address: test_contract_address!(
                    "0x046e9237f5408b5f899e72125dd69bd55485a287aaf24663d3ebe00d237fc7ef"
                ),
                class_hash: test_class_hash!("0x0358663e6ed9d37efd33d4661e20b2bad143e0f92076b0c91fe65f31ccf55046"),
                selector: Some(EntryPointSelector(test_felt!(
                    "0x00e5b455a836c7a254df57ed39d023d46b641b331162c6c0b369647056655409"
                ))),
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

    #[test]
    fn test_revert_error_keeps_all_tracebacks_for_call_contract_chain_with_string_error() {
        // Test case: CC(0) -> CC(1) -> StringFrame (no LibraryCall)
        // All tracebacks should be kept because there's no LibraryCall -
        // both CallContracts are at the end of the chain before the error.
        let mut error_stack = ErrorStack { header: ErrorStackHeader::Execution, stack: vec![] };

        // Entry 0: CallContract with VM traceback (should be kept - last before error)
        error_stack.push(
            EntryPointErrorFrame {
                depth: 0,
                preamble_type: PreambleType::CallContract,
                storage_address: test_contract_address!(
                    "0x01f062c02ee674cc7a88dd94e0b230b76decf76aff55b83ec32a90936e7569ab"
                ),
                class_hash: test_class_hash!("0x073414441639dcd11d1846f287650a00c60c416b9d3ba45d31c651672125b2c2"),
                selector: Some(EntryPointSelector(test_felt!(
                    "0x015d40a3d6ca2ac30f4031e42be28da9b056fef9bb7357ac5e85627ee876e5ad"
                ))),
            }
            .into(),
        );
        error_stack.push(
            VmExceptionFrame {
                pc: Relocatable { segment_index: 0, offset: 35988 },
                error_attr_value: None,
                traceback: Some("Cairo traceback (most recent call last):\nUnknown location (pc=0:330)\nUnknown location (pc=0:11695)\n".to_string()),
            }
            .into(),
        );

        // Entry 1: CallContract with VM traceback (should be kept - last before error)
        error_stack.push(
            EntryPointErrorFrame {
                depth: 1,
                preamble_type: PreambleType::CallContract,
                storage_address: test_contract_address!(
                    "0x02953d14869a4f634e02272ac288713dc514bfd018857569252b74f4a96e91fc"
                ),
                class_hash: test_class_hash!("0x05e4b69d808cd273b7d84ea27f1954c1eb8b61211036d293b1a0d5e9f34726e8"),
                selector: Some(EntryPointSelector(test_felt!(
                    "0x00aceca4cf913a062eea8c1609ce381630d82808d51e757d7b2b68c961933fa8"
                ))),
            }
            .into(),
        );
        error_stack.push(
            VmExceptionFrame {
                pc: Relocatable { segment_index: 0, offset: 117929 },
                error_attr_value: None,
                traceback: Some("Cairo traceback (most recent call last):\nUnknown location (pc=0:118178)\nUnknown location (pc=0:118178)\nUnknown location (pc=0:118178)\nUnknown location (pc=0:118178)\nUnknown location (pc=0:118178)\nUnknown location (pc=0:118178)\nUnknown location (pc=0:118178)\nUnknown location (pc=0:118178)\nUnknown location (pc=0:118178)\nUnknown location (pc=0:118178)\nUnknown location (pc=0:118178)\nUnknown location (pc=0:118178)\nUnknown location (pc=0:118178)\nUnknown location (pc=0:118178)\nUnknown location (pc=0:118178)\nUnknown location (pc=0:118178)\nUnknown location (pc=0:118178)\nUnknown location (pc=0:118178)\nUnknown location (pc=0:118178)\nUnknown location (pc=0:118178)\n".to_string()),
            }
            .into(),
        );

        // Final error message
        error_stack.push(ErrorStackSegment::StringFrame(
            "Exceeded the maximum number of events, number events: 1001, max number events: 1000.".to_string(),
        ));

        let revert_error = RevertError::Execution(error_stack);

        // Verify that the RevertError structure produces the correct input (unfiltered)
        let input = "Transaction execution has failed:\n0: Error in the called contract (contract address: 0x01f062c02ee674cc7a88dd94e0b230b76decf76aff55b83ec32a90936e7569ab, class hash: 0x073414441639dcd11d1846f287650a00c60c416b9d3ba45d31c651672125b2c2, selector: 0x015d40a3d6ca2ac30f4031e42be28da9b056fef9bb7357ac5e85627ee876e5ad):\nError at pc=0:35988:\nCairo traceback (most recent call last):\nUnknown location (pc=0:330)\nUnknown location (pc=0:11695)\n\n1: Error in the called contract (contract address: 0x02953d14869a4f634e02272ac288713dc514bfd018857569252b74f4a96e91fc, class hash: 0x05e4b69d808cd273b7d84ea27f1954c1eb8b61211036d293b1a0d5e9f34726e8, selector: 0x00aceca4cf913a062eea8c1609ce381630d82808d51e757d7b2b68c961933fa8):\nError at pc=0:117929:\nCairo traceback (most recent call last):\nUnknown location (pc=0:118178)\nUnknown location (pc=0:118178)\nUnknown location (pc=0:118178)\nUnknown location (pc=0:118178)\nUnknown location (pc=0:118178)\nUnknown location (pc=0:118178)\nUnknown location (pc=0:118178)\nUnknown location (pc=0:118178)\nUnknown location (pc=0:118178)\nUnknown location (pc=0:118178)\nUnknown location (pc=0:118178)\nUnknown location (pc=0:118178)\nUnknown location (pc=0:118178)\nUnknown location (pc=0:118178)\nUnknown location (pc=0:118178)\nUnknown location (pc=0:118178)\nUnknown location (pc=0:118178)\nUnknown location (pc=0:118178)\nUnknown location (pc=0:118178)\nUnknown location (pc=0:118178)\n\nExceeded the maximum number of events, number events: 1001, max number events: 1000.";
        assert_eq!(revert_error.to_string(), input);

        // Expected output: all tracebacks kept (entry 0 before LibraryCall, entry 1 belongs to LibraryCall, entry 2 before LibraryCall)
        let expected = "Transaction execution has failed:\n0: Error in the called contract (contract address: 0x01f062c02ee674cc7a88dd94e0b230b76decf76aff55b83ec32a90936e7569ab, class hash: 0x073414441639dcd11d1846f287650a00c60c416b9d3ba45d31c651672125b2c2, selector: 0x015d40a3d6ca2ac30f4031e42be28da9b056fef9bb7357ac5e85627ee876e5ad):\nError at pc=0:35988:\nCairo traceback (most recent call last):\nUnknown location (pc=0:330)\nUnknown location (pc=0:11695)\n\n1: Error in the called contract (contract address: 0x02953d14869a4f634e02272ac288713dc514bfd018857569252b74f4a96e91fc, class hash: 0x05e4b69d808cd273b7d84ea27f1954c1eb8b61211036d293b1a0d5e9f34726e8, selector: 0x00aceca4cf913a062eea8c1609ce381630d82808d51e757d7b2b68c961933fa8):\nError at pc=0:117929:\nCairo traceback (most recent call last):\nUnknown location (pc=0:118178)\nUnknown location (pc=0:118178)\nUnknown location (pc=0:118178)\nUnknown location (pc=0:118178)\nUnknown location (pc=0:118178)\nUnknown location (pc=0:118178)\nUnknown location (pc=0:118178)\nUnknown location (pc=0:118178)\nUnknown location (pc=0:118178)\nUnknown location (pc=0:118178)\nUnknown location (pc=0:118178)\nUnknown location (pc=0:118178)\nUnknown location (pc=0:118178)\nUnknown location (pc=0:118178)\nUnknown location (pc=0:118178)\nUnknown location (pc=0:118178)\nUnknown location (pc=0:118178)\nUnknown location (pc=0:118178)\nUnknown location (pc=0:118178)\nUnknown location (pc=0:118178)\n\nExceeded the maximum number of events, number events: 1001, max number events: 1000.";

        let result = revert_error.to_string();

        assert_eq!(result, expected);
    }

    #[test]
    fn test_revert_error_keeps_tracebacks_in_library_call_context_with_nested_call_chain() {
        // Test case: CC(0) -> LC(1, with VM) -> CC(2) -> CC(3) -> LC(4, no VM, only StringFrame)
        // All tracebacks should be kept because:
        // - Entry 0: First CC before LC (kept as first in chain)
        // - Entry 1: LibraryCall always keeps its traceback
        // - Entry 2: CC after LC context (has_library_call_before = true)
        // - Entry 3: Last CC before LC without VM
        let mut error_stack = ErrorStack { header: ErrorStackHeader::Execution, stack: vec![] };

        // Entry 0: CallContract with VM traceback (should be kept - next is LibraryCall)
        error_stack.push(
            EntryPointErrorFrame {
                depth: 0,
                preamble_type: PreambleType::CallContract,
                storage_address: test_contract_address!(
                    "0x07796f3bee40a288085b844d3a96a71d25e06d0dba9284a89196b8050fe5430f"
                ),
                class_hash: test_class_hash!("0x03530cc4759d78042f1b543bf797f5f3d647cde0388c33734cf91b7f7b9314a9"),
                selector: Some(EntryPointSelector(test_felt!(
                    "0x015d40a3d6ca2ac30f4031e42be28da9b056fef9bb7357ac5e85627ee876e5ad"
                ))),
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
                storage_address: test_contract_address!(
                    "0x07796f3bee40a288085b844d3a96a71d25e06d0dba9284a89196b8050fe5430f"
                ),
                class_hash: test_class_hash!("0x041cb0280ebadaa75f996d8d92c6f265f6d040bb3ba442e5f86a554f1765244e"),
                selector: Some(EntryPointSelector(test_felt!(
                    "0x015d40a3d6ca2ac30f4031e42be28da9b056fef9bb7357ac5e85627ee876e5ad"
                ))),
            }
            .into(),
        );
        error_stack.push(
            VmExceptionFrame {
                pc: Relocatable { segment_index: 0, offset: 56 },
                error_attr_value: None,
                traceback: Some("Cairo traceback (most recent call last):\nUnknown location (pc=0:1700)\nUnknown location (pc=0:1655)\nError message: multicall 1767341249246097106076784707040396935140562465068258416472332980274188751400 failed\nUnknown location (pc=0:179)\n".to_string()),
            }
            .into(),
        );

        // Entry 2: CallContract with VM traceback (should be kept - LibraryCall precedes and CallContract->LibraryCall follows)
        error_stack.push(
            EntryPointErrorFrame {
                depth: 2,
                preamble_type: PreambleType::CallContract,
                storage_address: test_contract_address!(
                    "0x0274d8800b7f4f60a13c8cf17fda9e949b099562195ab185ce667f2e483457c5"
                ),
                class_hash: test_class_hash!("0x0192c0791d74fe2667a9b7297b6aaaf1210da00374fae56debe13dc5cf1582af"),
                selector: Some(EntryPointSelector(test_felt!(
                    "0x03e8479a521791d4137c3f8bdcdd4d62d019135d1d86492d60476cee169b9228"
                ))),
            }
            .into(),
        );
        error_stack.push(
            VmExceptionFrame {
                pc: Relocatable { segment_index: 0, offset: 21216 },
                error_attr_value: None,
                traceback: Some("Cairo traceback (most recent call last):\nUnknown location (pc=0:6367)\nUnknown location (pc=0:11745)\nUnknown location (pc=0:17461)\n".to_string()),
            }
            .into(),
        );

        // Entry 3: CallContract with VM traceback (should be kept - next is LibraryCall)
        error_stack.push(
            EntryPointErrorFrame {
                depth: 3,
                preamble_type: PreambleType::CallContract,
                storage_address: test_contract_address!(
                    "0x07348407ebad690fec0cc8597e87dc16ef7b269a655ff72587dafff83d462be2"
                ),
                class_hash: test_class_hash!("0x070cdfaea3ec997bd3a8cdedfc0ffe804a58afc3d6b5a6e5c0218ec233ceea6d"),
                selector: Some(EntryPointSelector(test_felt!(
                    "0x03704ffe8fba161be0e994951751a5033b1462b918ff785c0a636be718dfdb68"
                ))),
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

        // Entry 4: LibraryCall with final error
        error_stack.push(
            EntryPointErrorFrame {
                depth: 4,
                preamble_type: PreambleType::LibraryCall,
                storage_address: test_contract_address!(
                    "0x07348407ebad690fec0cc8597e87dc16ef7b269a655ff72587dafff83d462be2"
                ),
                class_hash: test_class_hash!("0x05ffbcfeb50d200a0677c48a129a11245a3fc519d1d98d76882d1c9a1b19c6ed"),
                selector: Some(EntryPointSelector(test_felt!(
                    "0x03704ffe8fba161be0e994951751a5033b1462b918ff785c0a636be718dfdb68"
                ))),
            }
            .into(),
        );
        error_stack.push(
            ErrorStackSegment::StringFrame("Execution failed. Failure reason:\nError in contract (contract address: 0x07348407ebad690fec0cc8597e87dc16ef7b269a655ff72587dafff83d462be2, class hash: 0x05ffbcfeb50d200a0677c48a129a11245a3fc519d1d98d76882d1c9a1b19c6ed, selector: 0x03704ffe8fba161be0e994951751a5033b1462b918ff785c0a636be718dfdb68):\n0x753235365f737562204f766572666c6f77 ('u256_sub Overflow').\n".to_string()),
        );

        let revert_error = RevertError::Execution(error_stack);

        // Verify that the RevertError structure produces the correct input (unfiltered)
        let input = "Transaction execution has failed:\n0: Error in the called contract (contract address: 0x07796f3bee40a288085b844d3a96a71d25e06d0dba9284a89196b8050fe5430f, class hash: 0x03530cc4759d78042f1b543bf797f5f3d647cde0388c33734cf91b7f7b9314a9, selector: 0x015d40a3d6ca2ac30f4031e42be28da9b056fef9bb7357ac5e85627ee876e5ad):\nError at pc=0:12:\nCairo traceback (most recent call last):\nUnknown location (pc=0:161)\nUnknown location (pc=0:147)\n\n1: Error in a library call (contract address: 0x07796f3bee40a288085b844d3a96a71d25e06d0dba9284a89196b8050fe5430f, class hash: 0x041cb0280ebadaa75f996d8d92c6f265f6d040bb3ba442e5f86a554f1765244e, selector: 0x015d40a3d6ca2ac30f4031e42be28da9b056fef9bb7357ac5e85627ee876e5ad):\nError at pc=0:56:\nCairo traceback (most recent call last):\nUnknown location (pc=0:1700)\nUnknown location (pc=0:1655)\nError message: multicall 1767341249246097106076784707040396935140562465068258416472332980274188751400 failed\nUnknown location (pc=0:179)\n\n2: Error in the called contract (contract address: 0x0274d8800b7f4f60a13c8cf17fda9e949b099562195ab185ce667f2e483457c5, class hash: 0x0192c0791d74fe2667a9b7297b6aaaf1210da00374fae56debe13dc5cf1582af, selector: 0x03e8479a521791d4137c3f8bdcdd4d62d019135d1d86492d60476cee169b9228):\nError at pc=0:21216:\nCairo traceback (most recent call last):\nUnknown location (pc=0:6367)\nUnknown location (pc=0:11745)\nUnknown location (pc=0:17461)\n\n3: Error in the called contract (contract address: 0x07348407ebad690fec0cc8597e87dc16ef7b269a655ff72587dafff83d462be2, class hash: 0x070cdfaea3ec997bd3a8cdedfc0ffe804a58afc3d6b5a6e5c0218ec233ceea6d, selector: 0x03704ffe8fba161be0e994951751a5033b1462b918ff785c0a636be718dfdb68):\nError at pc=0:32:\nCairo traceback (most recent call last):\nUnknown location (pc=0:1683)\nUnknown location (pc=0:1669)\n\n4: Error in a library call (contract address: 0x07348407ebad690fec0cc8597e87dc16ef7b269a655ff72587dafff83d462be2, class hash: 0x05ffbcfeb50d200a0677c48a129a11245a3fc519d1d98d76882d1c9a1b19c6ed, selector: 0x03704ffe8fba161be0e994951751a5033b1462b918ff785c0a636be718dfdb68):\nExecution failed. Failure reason:\nError in contract (contract address: 0x07348407ebad690fec0cc8597e87dc16ef7b269a655ff72587dafff83d462be2, class hash: 0x05ffbcfeb50d200a0677c48a129a11245a3fc519d1d98d76882d1c9a1b19c6ed, selector: 0x03704ffe8fba161be0e994951751a5033b1462b918ff785c0a636be718dfdb68):\n0x753235365f737562204f766572666c6f77 ('u256_sub Overflow').\n";
        assert_eq!(revert_error.to_string(), input);

        // Expected output: all tracebacks kept (entry 0 before LibraryCall, entry 1 belongs to LibraryCall, entry 2 after LibraryCall before CallContract->LibraryCall, entry 3 before LibraryCall)
        let expected = "Transaction execution has failed:\n0: Error in the called contract (contract address: 0x07796f3bee40a288085b844d3a96a71d25e06d0dba9284a89196b8050fe5430f, class hash: 0x03530cc4759d78042f1b543bf797f5f3d647cde0388c33734cf91b7f7b9314a9, selector: 0x015d40a3d6ca2ac30f4031e42be28da9b056fef9bb7357ac5e85627ee876e5ad):\nError at pc=0:12:\nCairo traceback (most recent call last):\nUnknown location (pc=0:161)\nUnknown location (pc=0:147)\n\n1: Error in a library call (contract address: 0x07796f3bee40a288085b844d3a96a71d25e06d0dba9284a89196b8050fe5430f, class hash: 0x041cb0280ebadaa75f996d8d92c6f265f6d040bb3ba442e5f86a554f1765244e, selector: 0x015d40a3d6ca2ac30f4031e42be28da9b056fef9bb7357ac5e85627ee876e5ad):\nError at pc=0:56:\nCairo traceback (most recent call last):\nUnknown location (pc=0:1700)\nUnknown location (pc=0:1655)\nError message: multicall 1767341249246097106076784707040396935140562465068258416472332980274188751400 failed\nUnknown location (pc=0:179)\n\n2: Error in the called contract (contract address: 0x0274d8800b7f4f60a13c8cf17fda9e949b099562195ab185ce667f2e483457c5, class hash: 0x0192c0791d74fe2667a9b7297b6aaaf1210da00374fae56debe13dc5cf1582af, selector: 0x03e8479a521791d4137c3f8bdcdd4d62d019135d1d86492d60476cee169b9228):\nError at pc=0:21216:\nCairo traceback (most recent call last):\nUnknown location (pc=0:6367)\nUnknown location (pc=0:11745)\nUnknown location (pc=0:17461)\n\n3: Error in the called contract (contract address: 0x07348407ebad690fec0cc8597e87dc16ef7b269a655ff72587dafff83d462be2, class hash: 0x070cdfaea3ec997bd3a8cdedfc0ffe804a58afc3d6b5a6e5c0218ec233ceea6d, selector: 0x03704ffe8fba161be0e994951751a5033b1462b918ff785c0a636be718dfdb68):\nError at pc=0:32:\nCairo traceback (most recent call last):\nUnknown location (pc=0:1683)\nUnknown location (pc=0:1669)\n\n4: Error in a library call (contract address: 0x07348407ebad690fec0cc8597e87dc16ef7b269a655ff72587dafff83d462be2, class hash: 0x05ffbcfeb50d200a0677c48a129a11245a3fc519d1d98d76882d1c9a1b19c6ed, selector: 0x03704ffe8fba161be0e994951751a5033b1462b918ff785c0a636be718dfdb68):\nExecution failed. Failure reason:\nError in contract (contract address: 0x07348407ebad690fec0cc8597e87dc16ef7b269a655ff72587dafff83d462be2, class hash: 0x05ffbcfeb50d200a0677c48a129a11245a3fc519d1d98d76882d1c9a1b19c6ed, selector: 0x03704ffe8fba161be0e994951751a5033b1462b918ff785c0a636be718dfdb68):\n0x753235365f737562204f766572666c6f77 ('u256_sub Overflow').\n";

        let result = revert_error.format_for_receipt().to_string();

        assert_eq!(result, expected);
    }

    #[test]
    fn test_revert_error_filters_call_contract_chain_when_library_call_has_own_vm() {
        // Test case: CC(0) -> CC(1) -> LC(2, with VM) -> LC(3, no VM, only StringFrame)
        // Entry 0 and Entry 1 VM tracebacks should be removed because:
        // - The LibraryCall at entry 2 has its own VM traceback
        // - Entry 1 is not the first CC in the chain (entry 0 precedes it)
        // - Entry 0 is followed by another CC, not directly by LC
        // Only the LibraryCall's VM traceback (entry 2) is kept.

        let mut error_stack = ErrorStack { header: ErrorStackHeader::Execution, stack: vec![] };

        // Entry 0: CallContract with VM traceback (should be removed - followed by another CallContract)
        error_stack.push(
            EntryPointErrorFrame {
                depth: 0,
                preamble_type: PreambleType::CallContract,
                storage_address: test_contract_address!(
                    "0x051a24146bfe38f21f6a119443a071710e28c05981e854394c7a72ec2b729c2c"
                ),
                class_hash: test_class_hash!("0x073414441639dcd11d1846f287650a00c60c416b9d3ba45d31c651672125b2c2"),
                selector: Some(EntryPointSelector(test_felt!(
                    "0x015d40a3d6ca2ac30f4031e42be28da9b056fef9bb7357ac5e85627ee876e5ad"
                ))),
            }
            .into(),
        );
        error_stack.push(
            VmExceptionFrame {
                pc: Relocatable { segment_index: 0, offset: 35988 },
                error_attr_value: None,
                traceback: Some("Cairo traceback (most recent call last):\nUnknown location (pc=0:330)\nUnknown location (pc=0:11695)\n".to_string()),
            }
            .into(),
        );

        // Entry 1: CallContract with VM traceback (should be removed - next is LibraryCall but traceback is redundant)
        error_stack.push(
            EntryPointErrorFrame {
                depth: 1,
                preamble_type: PreambleType::CallContract,
                storage_address: test_contract_address!(
                    "0x012f3ff23c30ac1cd01cd6a929eb18553dbf0e2ed84fa999781707617eef6720"
                ),
                class_hash: test_class_hash!("0x05755c007b188110107d00696dcc349b158da075d395b1414fd5c6662a9d44a8"),
                selector: Some(EntryPointSelector(test_felt!(
                    "0x01c2961b7317d2486c600e15b77a7261b34e53ad3f0faa22d71320bbe7709ae2"
                ))),
            }
            .into(),
        );
        error_stack.push(
            VmExceptionFrame {
                pc: Relocatable { segment_index: 0, offset: 371 },
                error_attr_value: None,
                traceback: Some("Cairo traceback (most recent call last):\nUnknown location (pc=0:155)\n".to_string()),
            }
            .into(),
        );

        // Entry 2: LibraryCall with VM traceback (should be kept - belongs to LibraryCall)
        error_stack.push(
            EntryPointErrorFrame {
                depth: 2,
                preamble_type: PreambleType::LibraryCall,
                storage_address: test_contract_address!(
                    "0x012f3ff23c30ac1cd01cd6a929eb18553dbf0e2ed84fa999781707617eef6720"
                ),
                class_hash: test_class_hash!("0x070cdfaea3ec997bd3a8cdedfc0ffe804a58afc3d6b5a6e5c0218ec233ceea6d"),
                selector: Some(EntryPointSelector(test_felt!(
                    "0x0000000000000000000000000000000000000000000000000000000000000abc"
                ))),
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

        // Entry 3: LibraryCall with StringFrame error (no VM traceback)
        error_stack.push(
            EntryPointErrorFrame {
                depth: 3,
                preamble_type: PreambleType::LibraryCall,
                storage_address: test_contract_address!(
                    "0x012f3ff23c30ac1cd01cd6a929eb18553dbf0e2ed84fa999781707617eef6720"
                ),
                class_hash: test_class_hash!("0x0000000000000000000000000000000000000000000000000000000000000000"),
                selector: Some(EntryPointSelector(test_felt!(
                    "0x0000000000000000000000000000000000000000000000000000000000000abc"
                ))),
            }
            .into(),
        );
        error_stack.push(ErrorStackSegment::StringFrame(
            "Class with hash 0x0000000000000000000000000000000000000000000000000000000000000000 is not declared.\n"
                .to_string(),
        ));

        let revert_error = RevertError::Execution(error_stack);

        // Verify that the RevertError structure produces the correct input (unfiltered - what blockifier produces)
        let input = "Transaction execution has failed:\n0: Error in the called contract (contract address: 0x051a24146bfe38f21f6a119443a071710e28c05981e854394c7a72ec2b729c2c, class hash: 0x073414441639dcd11d1846f287650a00c60c416b9d3ba45d31c651672125b2c2, selector: 0x015d40a3d6ca2ac30f4031e42be28da9b056fef9bb7357ac5e85627ee876e5ad):\nError at pc=0:35988:\nCairo traceback (most recent call last):\nUnknown location (pc=0:330)\nUnknown location (pc=0:11695)\n\n1: Error in the called contract (contract address: 0x012f3ff23c30ac1cd01cd6a929eb18553dbf0e2ed84fa999781707617eef6720, class hash: 0x05755c007b188110107d00696dcc349b158da075d395b1414fd5c6662a9d44a8, selector: 0x01c2961b7317d2486c600e15b77a7261b34e53ad3f0faa22d71320bbe7709ae2):\nError at pc=0:371:\nCairo traceback (most recent call last):\nUnknown location (pc=0:155)\n\n2: Error in a library call (contract address: 0x012f3ff23c30ac1cd01cd6a929eb18553dbf0e2ed84fa999781707617eef6720, class hash: 0x070cdfaea3ec997bd3a8cdedfc0ffe804a58afc3d6b5a6e5c0218ec233ceea6d, selector: 0x0000000000000000000000000000000000000000000000000000000000000abc):\nError at pc=0:32:\nCairo traceback (most recent call last):\nUnknown location (pc=0:1683)\nUnknown location (pc=0:1669)\n\n3: Error in a library call (contract address: 0x012f3ff23c30ac1cd01cd6a929eb18553dbf0e2ed84fa999781707617eef6720, class hash: 0x0000000000000000000000000000000000000000000000000000000000000000, selector: 0x0000000000000000000000000000000000000000000000000000000000000abc):\nClass with hash 0x0000000000000000000000000000000000000000000000000000000000000000 is not declared.\n";
        assert_eq!(revert_error.to_string(), input);

        // Expected output: VM tracebacks from entry 0 and entry 1 are removed, only entry 2's traceback is kept
        let expected = "Transaction execution has failed:\n0: Error in the called contract (contract address: 0x051a24146bfe38f21f6a119443a071710e28c05981e854394c7a72ec2b729c2c, class hash: 0x073414441639dcd11d1846f287650a00c60c416b9d3ba45d31c651672125b2c2, selector: 0x015d40a3d6ca2ac30f4031e42be28da9b056fef9bb7357ac5e85627ee876e5ad):\n1: Error in the called contract (contract address: 0x012f3ff23c30ac1cd01cd6a929eb18553dbf0e2ed84fa999781707617eef6720, class hash: 0x05755c007b188110107d00696dcc349b158da075d395b1414fd5c6662a9d44a8, selector: 0x01c2961b7317d2486c600e15b77a7261b34e53ad3f0faa22d71320bbe7709ae2):\n2: Error in a library call (contract address: 0x012f3ff23c30ac1cd01cd6a929eb18553dbf0e2ed84fa999781707617eef6720, class hash: 0x070cdfaea3ec997bd3a8cdedfc0ffe804a58afc3d6b5a6e5c0218ec233ceea6d, selector: 0x0000000000000000000000000000000000000000000000000000000000000abc):\nError at pc=0:32:\nCairo traceback (most recent call last):\nUnknown location (pc=0:1683)\nUnknown location (pc=0:1669)\n\n3: Error in a library call (contract address: 0x012f3ff23c30ac1cd01cd6a929eb18553dbf0e2ed84fa999781707617eef6720, class hash: 0x0000000000000000000000000000000000000000000000000000000000000000, selector: 0x0000000000000000000000000000000000000000000000000000000000000abc):\nClass with hash 0x0000000000000000000000000000000000000000000000000000000000000000 is not declared.\n";

        let result = revert_error.format_for_receipt().to_string();

        assert_eq!(result, expected);
    }
    
    #[test]
    fn test_format_for_receipt_filters_callcontract_before_librarycall() {
        // This test case represents the actual error structure observed in production:
        // - Entry 0: CallContract (VM should be filtered - has more entry points after)
        // - Entry 1: CallContract (VM should be filtered - has more entry points after)
        // - Entry 2: LibraryCall (VM should be kept - LibraryCall always keeps VM)
        // - Entry 3: LibraryCall with StringFrame (final error, no VM)
        //
        // This matches the Starkware sequencer behavior where CallContract VM tracebacks
        // are filtered when there are more entry points in the call stack.
        let mut error_stack = ErrorStack { header: ErrorStackHeader::Execution, stack: vec![] };

        // Entry 0: CallContract with VM traceback (should be filtered)
        error_stack.push(
            EntryPointErrorFrame {
                depth: 0,
                preamble_type: PreambleType::CallContract,
                storage_address: test_contract_address!(
                    "0x051a24146bfe38f21f6a119443a071710e28c05981e854394c7a72ec2b729c2c"
                ),
                class_hash: test_class_hash!("0x073414441639dcd11d1846f287650a00c60c416b9d3ba45d31c651672125b2c2"),
                selector: Some(EntryPointSelector(test_felt!(
                    "0x015d40a3d6ca2ac30f4031e42be28da9b056fef9bb7357ac5e85627ee876e5ad"
                ))),
            }
            .into(),
        );
        error_stack.push(
            VmExceptionFrame {
                pc: Relocatable { segment_index: 0, offset: 35988 },
                error_attr_value: None,
                traceback: Some("Cairo traceback (most recent call last):\nUnknown location (pc=0:330)\nUnknown location (pc=0:11695)\nUnknown location (pc=0:36001)\nUnknown location (pc=0:36001)\nUnknown location (pc=0:36001)\nUnknown location (pc=0:36001)\nUnknown location (pc=0:36001)\n".to_string()),
            }
            .into(),
        );

        // Entry 1: CallContract with VM traceback (should be filtered - next is LibraryCall but there are more entry points)
        error_stack.push(
            EntryPointErrorFrame {
                depth: 1,
                preamble_type: PreambleType::CallContract,
                storage_address: test_contract_address!(
                    "0x062834ad1c4f52429e246bdf24055963ac7ae388ed50e91117c0da5ad9eb8984"
                ),
                class_hash: test_class_hash!("0x05755c007b188110107d00696dcc349b158da075d395b1414fd5c6662a9d44a8"),
                selector: Some(EntryPointSelector(test_felt!(
                    "0x01c2961b7317d2486c600e15b77a7261b34e53ad3f0faa22d71320bbe7709ae2"
                ))),
            }
            .into(),
        );
        error_stack.push(
            VmExceptionFrame {
                pc: Relocatable { segment_index: 0, offset: 371 },
                error_attr_value: None,
                traceback: Some("Cairo traceback (most recent call last):\nUnknown location (pc=0:155)\n".to_string()),
            }
            .into(),
        );

        // Entry 2: LibraryCall with VM traceback (should be kept - LibraryCall)
        error_stack.push(
            EntryPointErrorFrame {
                depth: 2,
                preamble_type: PreambleType::LibraryCall,
                storage_address: test_contract_address!(
                    "0x062834ad1c4f52429e246bdf24055963ac7ae388ed50e91117c0da5ad9eb8984"
                ),
                class_hash: test_class_hash!("0x070cdfaea3ec997bd3a8cdedfc0ffe804a58afc3d6b5a6e5c0218ec233ceea6d"),
                selector: Some(EntryPointSelector(test_felt!(
                    "0x0000000000000000000000000000000000000000000000000000000000000abc"
                ))),
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

        // Entry 3: LibraryCall with final error (no VM traceback, just StringFrame)
        error_stack.push(
            EntryPointErrorFrame {
                depth: 3,
                preamble_type: PreambleType::LibraryCall,
                storage_address: test_contract_address!(
                    "0x062834ad1c4f52429e246bdf24055963ac7ae388ed50e91117c0da5ad9eb8984"
                ),
                class_hash: test_class_hash!("0x0000000000000000000000000000000000000000000000000000000000000000"),
                selector: Some(EntryPointSelector(test_felt!(
                    "0x0000000000000000000000000000000000000000000000000000000000000abc"
                ))),
            }
            .into(),
        );
        error_stack.push(ErrorStackSegment::StringFrame(
            "Class with hash 0x0000000000000000000000000000000000000000000000000000000000000000 is not declared.\n"
                .to_string(),
        ));

        let revert_error = RevertError::Execution(error_stack);

        // Verify the unfiltered input (what blockifier produces)
        let input = "Transaction execution has failed:\n0: Error in the called contract (contract address: 0x051a24146bfe38f21f6a119443a071710e28c05981e854394c7a72ec2b729c2c, class hash: 0x073414441639dcd11d1846f287650a00c60c416b9d3ba45d31c651672125b2c2, selector: 0x015d40a3d6ca2ac30f4031e42be28da9b056fef9bb7357ac5e85627ee876e5ad):\nError at pc=0:35988:\nCairo traceback (most recent call last):\nUnknown location (pc=0:330)\nUnknown location (pc=0:11695)\nUnknown location (pc=0:36001)\nUnknown location (pc=0:36001)\nUnknown location (pc=0:36001)\nUnknown location (pc=0:36001)\nUnknown location (pc=0:36001)\n\n1: Error in the called contract (contract address: 0x062834ad1c4f52429e246bdf24055963ac7ae388ed50e91117c0da5ad9eb8984, class hash: 0x05755c007b188110107d00696dcc349b158da075d395b1414fd5c6662a9d44a8, selector: 0x01c2961b7317d2486c600e15b77a7261b34e53ad3f0faa22d71320bbe7709ae2):\nError at pc=0:371:\nCairo traceback (most recent call last):\nUnknown location (pc=0:155)\n\n2: Error in a library call (contract address: 0x062834ad1c4f52429e246bdf24055963ac7ae388ed50e91117c0da5ad9eb8984, class hash: 0x070cdfaea3ec997bd3a8cdedfc0ffe804a58afc3d6b5a6e5c0218ec233ceea6d, selector: 0x0000000000000000000000000000000000000000000000000000000000000abc):\nError at pc=0:32:\nCairo traceback (most recent call last):\nUnknown location (pc=0:1683)\nUnknown location (pc=0:1669)\n\n3: Error in a library call (contract address: 0x062834ad1c4f52429e246bdf24055963ac7ae388ed50e91117c0da5ad9eb8984, class hash: 0x0000000000000000000000000000000000000000000000000000000000000000, selector: 0x0000000000000000000000000000000000000000000000000000000000000abc):\nClass with hash 0x0000000000000000000000000000000000000000000000000000000000000000 is not declared.\n";
        assert_eq!(revert_error.to_string(), input);

        // Expected output: matches Starkware sequencer behavior
        // - Entry 0 CallContract VM: filtered (has entry points after)
        // - Entry 1 CallContract VM: filtered (has entry points after)
        // - Entry 2 LibraryCall VM: kept (LibraryCall)
        // - Entry 3 LibraryCall: just the error string
        let expected = "Transaction execution has failed:\n0: Error in the called contract (contract address: 0x051a24146bfe38f21f6a119443a071710e28c05981e854394c7a72ec2b729c2c, class hash: 0x073414441639dcd11d1846f287650a00c60c416b9d3ba45d31c651672125b2c2, selector: 0x015d40a3d6ca2ac30f4031e42be28da9b056fef9bb7357ac5e85627ee876e5ad):\n1: Error in the called contract (contract address: 0x062834ad1c4f52429e246bdf24055963ac7ae388ed50e91117c0da5ad9eb8984, class hash: 0x05755c007b188110107d00696dcc349b158da075d395b1414fd5c6662a9d44a8, selector: 0x01c2961b7317d2486c600e15b77a7261b34e53ad3f0faa22d71320bbe7709ae2):\n2: Error in a library call (contract address: 0x062834ad1c4f52429e246bdf24055963ac7ae388ed50e91117c0da5ad9eb8984, class hash: 0x070cdfaea3ec997bd3a8cdedfc0ffe804a58afc3d6b5a6e5c0218ec233ceea6d, selector: 0x0000000000000000000000000000000000000000000000000000000000000abc):\nError at pc=0:32:\nCairo traceback (most recent call last):\nUnknown location (pc=0:1683)\nUnknown location (pc=0:1669)\n\n3: Error in a library call (contract address: 0x062834ad1c4f52429e246bdf24055963ac7ae388ed50e91117c0da5ad9eb8984, class hash: 0x0000000000000000000000000000000000000000000000000000000000000000, selector: 0x0000000000000000000000000000000000000000000000000000000000000abc):\nClass with hash 0x0000000000000000000000000000000000000000000000000000000000000000 is not declared.\n";

        let result = revert_error.format_for_receipt().to_string();

        assert_eq!(result, expected);
    }
}
