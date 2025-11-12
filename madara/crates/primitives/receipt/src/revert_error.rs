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
///   - Keep the traceback if the next entry is a LibraryCall or if it's the last entry
///   - Remove the traceback if the next entry is another CallContract
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
                !has_nested_call_contract_after(error_stack, vm_index)
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

    ErrorStack {
        header: error_stack.header,
        stack: stack_vec,
    }
}

/// Filters redundant VM tracebacks from the error stack (reference version for display).
///
/// This version works with references and constructs a new ErrorStack for display purposes.
/// Used when we can't move out of the original error stack.
fn filter_redundant_vm_tracebacks_ref(error_stack: &ErrorStack) -> ErrorStack {
    let mut new_stack = ErrorStack {
        header: error_stack.header.clone(),
        stack: Vec::new(),
    };

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
                    }
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
    fn test_format_for_receipt_filters_redundant_tracebacks() {
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
                storage_address: test_contract_address!("0x05743c833ed33a3433f1c5587dac97753ddcc84f9844e6fa2a3268e5ae35cbc3"),
                class_hash: test_class_hash!("0x073414441639dcd11d1846f287650a00c60c416b9d3ba45d31c651672125b2c2"),
                selector: Some(EntryPointSelector(test_felt!("0x015d40a3d6ca2ac30f4031e42be28da9b056fef9bb7357ac5e85627ee876e5ad"))),
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
                storage_address: test_contract_address!("0x0286003f7c7bfc3f94e8f0af48b48302e7aee2fb13c23b141479ba00832ef2c6"),
                class_hash: test_class_hash!("0x03e283b1e8bce178469acb94700999ecc7ad180420201e16eb0a81294ae8599b"),
                selector: Some(EntryPointSelector(test_felt!("0x0056878e39e16b42520b0d7936d3fd3498f86ceda4dbad50f6ff717644c95ed6"))),
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
                storage_address: test_contract_address!("0x06f373b346561036d98ea10fb3e60d2f459c872b1933b50b21fe6ef4fda3b75e"),
                class_hash: test_class_hash!("0x070cdfaea3ec997bd3a8cdedfc0ffe804a58afc3d6b5a6e5c0218ec233ceea6d"),
                selector: Some(EntryPointSelector(test_felt!("0x0041b033f4a31df8067c24d1e9b550a2ce75fd4a29e1147af9752174f0e6cb20"))),
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
                storage_address: test_contract_address!("0x06f373b346561036d98ea10fb3e60d2f459c872b1933b50b21fe6ef4fda3b75e"),
                class_hash: test_class_hash!("0x05ffbcfeb50d200a0677c48a129a11245a3fc519d1d98d76882d1c9a1b19c6ed"),
                selector: Some(EntryPointSelector(test_felt!("0x0041b033f4a31df8067c24d1e9b550a2ce75fd4a29e1147af9752174f0e6cb20"))),
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
                storage_address: test_contract_address!("0x006fb38baf7a14acc032ff556c2791b03292861581572d02296c5093fd16cafb"),
                class_hash: test_class_hash!("0x03530cc4759d78042f1b543bf797f5f3d647cde0388c33734cf91b7f7b9314a9"),
                selector: Some(EntryPointSelector(test_felt!("0x015d40a3d6ca2ac30f4031e42be28da9b056fef9bb7357ac5e85627ee876e5ad"))),
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
                storage_address: test_contract_address!("0x006fb38baf7a14acc032ff556c2791b03292861581572d02296c5093fd16cafb"),
                class_hash: test_class_hash!("0x041cb0280ebadaa75f996d8d92c6f265f6d040bb3ba442e5f86a554f1765244e"),
                selector: Some(EntryPointSelector(test_felt!("0x015d40a3d6ca2ac30f4031e42be28da9b056fef9bb7357ac5e85627ee876e5ad"))),
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
                storage_address: test_contract_address!("0x046e9237f5408b5f899e72125dd69bd55485a287aaf24663d3ebe00d237fc7ef"),
                class_hash: test_class_hash!("0x070cdfaea3ec997bd3a8cdedfc0ffe804a58afc3d6b5a6e5c0218ec233ceea6d"),
                selector: Some(EntryPointSelector(test_felt!("0x00e5b455a836c7a254df57ed39d023d46b641b331162c6c0b369647056655409"))),
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
                storage_address: test_contract_address!("0x046e9237f5408b5f899e72125dd69bd55485a287aaf24663d3ebe00d237fc7ef"),
                class_hash: test_class_hash!("0x0358663e6ed9d37efd33d4661e20b2bad143e0f92076b0c91fe65f31ccf55046"),
                selector: Some(EntryPointSelector(test_felt!("0x00e5b455a836c7a254df57ed39d023d46b641b331162c6c0b369647056655409"))),
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
