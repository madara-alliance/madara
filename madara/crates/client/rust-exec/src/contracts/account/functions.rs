//! Account contract function implementations.
//!
//! This module implements the core account functions:
//! - __validate__: Verify transaction signature
//! - __execute__: Execute multicall

use starknet_types_core::felt::Felt;

use super::layout;
use crate::context::ExecutionContext;
use crate::contracts::ExecutionError;
use crate::state::StateReader;
use crate::types::ContractAddress;

/// A call to be executed by the account.
#[derive(Debug, Clone)]
pub struct Call {
    pub to: ContractAddress,
    pub selector: Felt,
    pub calldata: Vec<Felt>,
}

/// Parse calls from calldata.
///
/// The calldata format for __execute__ is:
/// [num_calls, call_0_to, call_0_selector, call_0_data_len, call_0_data..., call_1_to, ...]
pub fn parse_calls(calldata: &[Felt]) -> Result<Vec<Call>, ExecutionError> {
    if calldata.is_empty() {
        return Err(ExecutionError::ExecutionFailed("Empty calldata".to_string()));
    }

    let num_calls = felt_to_usize(calldata[0])?;
    let mut calls = Vec::with_capacity(num_calls);
    let mut offset = 1;

    for _ in 0..num_calls {
        if offset + 2 >= calldata.len() {
            return Err(ExecutionError::ExecutionFailed("Invalid calldata: truncated".to_string()));
        }

        let to = ContractAddress(calldata[offset]);
        let selector = calldata[offset + 1];
        let data_len = felt_to_usize(calldata[offset + 2])?;
        offset += 3;

        if offset + data_len > calldata.len() {
            return Err(ExecutionError::ExecutionFailed("Invalid calldata: data truncated".to_string()));
        }

        let data = calldata[offset..offset + data_len].to_vec();
        offset += data_len;

        calls.push(Call { to, selector, calldata: data });
    }

    Ok(calls)
}

/// Execute __validate__ function.
///
/// This validates the transaction signature against the account's public key.
/// For simplicity, we assume validation always passes (signature verification
/// would require ECDSA implementation).
pub fn execute_validate<S: StateReader>(
    state: &S,
    account_address: ContractAddress,
    _tx_hash: Felt,
    signature: &[Felt],
    ctx: &mut ExecutionContext,
) -> Result<(), ExecutionError> {
    // Read the account's public key
    let public_key = ctx.storage_read(state, account_address, *layout::ACCOUNT_PUBLIC_KEY)?;

    // In a full implementation, we would:
    // 1. Verify signature.len() == 2 (r, s)
    // 2. Verify ECDSA signature: verify(public_key, tx_hash, signature)
    //
    // For now, we just check that a public key exists and signature is provided
    if public_key == Felt::ZERO {
        return Err(ExecutionError::ExecutionFailed("Account has no public key".to_string()));
    }

    if signature.len() < 2 {
        return Err(ExecutionError::ExecutionFailed("Invalid signature length".to_string()));
    }

    // Signature verification would go here
    // For now, we trust that Blockifier already validated it

    // Return VALID (as expected by the protocol)
    ctx.set_retdata(vec![Felt::from_hex_unchecked("0x56414c4944")]); // "VALID" as felt

    Ok(())
}

/// Execute __execute__ function.
///
/// This parses the calls from calldata and executes them.
/// Returns the results from all calls.
///
/// NOTE: For verification purposes, we DON'T merge nested contract storage/events
/// into the account's result. Blockifier traces each nested call separately,
/// so those are verified independently. The account's execution result should
/// only contain the account's own state changes (if any).
pub fn execute_execute<S: StateReader>(
    state: &S,
    _account_address: ContractAddress,
    calldata: &[Felt],
    ctx: &mut ExecutionContext,
) -> Result<Vec<Vec<Felt>>, ExecutionError> {
    // Parse calls from calldata
    let calls = parse_calls(calldata)?;

    let mut all_results = Vec::new();

    // Execute each call to get retdata, but DON'T merge storage/events
    // Those are verified separately for each nested contract
    for call in &calls {
        // Get the class hash of the target contract
        let class_hash = state
            .get_class_hash_at(call.to)?
            .ok_or_else(|| ExecutionError::ExecutionFailed(format!("Contract not deployed: {:?}", call.to)))?;

        // Try to execute with our Rust implementation
        let result = if let Some(exec_result) = crate::contracts::ContractRegistry::execute(
            state,
            call.to,
            class_hash,
            call.selector,
            &call.calldata,
            call.to, // caller is the contract itself for nested calls
        ) {
            let result = exec_result?;
            // Only collect retdata - storage/events are traced separately
            result.call_result.retdata
        } else {
            // Contract not supported in Rust - would need to call Blockifier
            // For verification purposes, we skip unsupported contracts
            vec![]
        };

        all_results.push(result);
    }

    // The account's __execute__ typically returns empty retdata in traces
    // because the actual return data is captured in the nested call traces.
    // Blockifier's trace returns [] for the account's __execute__.
    ctx.set_retdata(vec![]);

    Ok(all_results)
}

/// Convert Felt to usize.
fn felt_to_usize(felt: Felt) -> Result<usize, ExecutionError> {
    let bytes = felt.to_bytes_be();
    // Check if value fits in usize (first 24 bytes should be zero)
    if bytes.iter().take(24).any(|&b| b != 0) {
        return Err(ExecutionError::ExecutionFailed("Value too large".to_string()));
    }
    let mut arr = [0u8; 8];
    arr.copy_from_slice(&bytes[24..32]);
    Ok(u64::from_be_bytes(arr) as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_single_call() {
        // [1, to, selector, 2, data0, data1]
        let calldata = vec![
            Felt::from(1u64),     // num_calls
            Felt::from(0x123u64), // to
            Felt::from(0x456u64), // selector
            Felt::from(2u64),     // data_len
            Felt::from(0x789u64), // data[0]
            Felt::from(0xabcu64), // data[1]
        ];

        let calls = parse_calls(&calldata).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].to.0, Felt::from(0x123u64));
        assert_eq!(calls[0].selector, Felt::from(0x456u64));
        assert_eq!(calls[0].calldata.len(), 2);
    }

    #[test]
    fn test_parse_multiple_calls() {
        // [2, call1..., call2...]
        let calldata = vec![
            Felt::from(2u64), // num_calls
            // Call 1
            Felt::from(0x111u64), // to
            Felt::from(0x222u64), // selector
            Felt::from(0u64),     // data_len
            // Call 2
            Felt::from(0x333u64), // to
            Felt::from(0x444u64), // selector
            Felt::from(1u64),     // data_len
            Felt::from(0x555u64), // data[0]
        ];

        let calls = parse_calls(&calldata).unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].to.0, Felt::from(0x111u64));
        assert_eq!(calls[1].to.0, Felt::from(0x333u64));
        assert_eq!(calls[1].calldata.len(), 1);
    }
}
