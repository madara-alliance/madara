//! ERC20 function implementations.
//!
//! This module implements the core ERC20 functions used for fee token operations.

use starknet_types_core::felt::Felt;

use super::layout;
use crate::context::ExecutionContext;
use crate::contracts::ExecutionError;
use crate::state::StateReader;
use crate::types::{ContractAddress, StateDiff};

/// Read a u256 balance from storage.
///
/// ERC20 balances are stored as u256 (two felts: low and high).
pub fn read_balance<S: StateReader>(
    state: &S,
    token_address: ContractAddress,
    owner: ContractAddress,
    ctx: &mut ExecutionContext,
) -> Result<(u128, u128), ExecutionError> {
    let key_low = layout::balance_key(owner);
    let key_high = layout::balance_key_high(owner);

    let low = ctx.storage_read(state, token_address, key_low)?;
    let high = ctx.storage_read(state, token_address, key_high)?;

    // Convert felts to u128
    let low_u128 = felt_to_u128(low)?;
    let high_u128 = felt_to_u128(high)?;

    Ok((low_u128, high_u128))
}

/// Write a u256 balance to storage.
pub fn write_balance(
    ctx: &mut ExecutionContext,
    token_address: ContractAddress,
    owner: ContractAddress,
    low: u128,
    high: u128,
) {
    let key_low = layout::balance_key(owner);
    let key_high = layout::balance_key_high(owner);

    ctx.storage_write(token_address, key_low, Felt::from(low));
    ctx.storage_write(token_address, key_high, Felt::from(high));
}

/// Execute balance_of function.
pub fn execute_balance_of<S: StateReader>(
    state: &S,
    token_address: ContractAddress,
    owner: ContractAddress,
    ctx: &mut ExecutionContext,
) -> Result<(), ExecutionError> {
    let (low, high) = read_balance(state, token_address, owner, ctx)?;

    // Return u256 as two felts (low, high)
    ctx.set_retdata(vec![Felt::from(low), Felt::from(high)]);
    Ok(())
}

/// Execute transfer function.
///
/// Transfers `amount` tokens from `from` to `to`.
/// This is used for fee transfers where we bypass approval checks.
pub fn execute_transfer<S: StateReader>(
    state: &S,
    token_address: ContractAddress,
    from: ContractAddress,
    to: ContractAddress,
    amount_low: u128,
    amount_high: u128,
    ctx: &mut ExecutionContext,
) -> Result<(), ExecutionError> {
    // Read sender balance
    let (from_balance_low, from_balance_high) = read_balance(state, token_address, from, ctx)?;

    // Check sufficient balance (simplified - assumes amount fits in low)
    if amount_high > 0 {
        return Err(ExecutionError::ExecutionFailed("Amount too large".to_string()));
    }

    let from_balance = u256_from_parts(from_balance_low, from_balance_high);
    let amount = amount_low;

    if from_balance < amount {
        return Err(ExecutionError::ExecutionFailed(format!(
            "Insufficient balance: have {}, need {}",
            from_balance, amount
        )));
    }

    // Calculate new balances
    let new_from_balance = from_balance - amount;
    let (new_from_low, new_from_high) = u256_to_parts(new_from_balance);

    // Read recipient balance
    let (to_balance_low, to_balance_high) = read_balance(state, token_address, to, ctx)?;
    let to_balance = u256_from_parts(to_balance_low, to_balance_high);
    let new_to_balance = to_balance + amount;
    let (new_to_low, new_to_high) = u256_to_parts(new_to_balance);

    // Write new balances
    write_balance(ctx, token_address, from, new_from_low, new_from_high);
    write_balance(ctx, token_address, to, new_to_low, new_to_high);

    // Emit Transfer event
    // Transfer event: keys = [selector, from, to], data = [amount_low, amount_high]
    let transfer_selector = crate::storage::sn_keccak(b"Transfer");
    ctx.emit_event(vec![transfer_selector, from.0, to.0], vec![Felt::from(amount_low), Felt::from(amount_high)]);

    // Return success (true as felt)
    ctx.set_retdata(vec![Felt::ONE]);

    Ok(())
}

/// Internal transfer without emitting events (for fee transfers).
///
/// This directly updates storage without going through the full ERC20 logic.
pub fn transfer_internal_direct<S: StateReader>(
    state: &S,
    token_address: ContractAddress,
    from: ContractAddress,
    to: ContractAddress,
    amount: u128,
    state_diff: &mut StateDiff,
) -> Result<(), ExecutionError> {
    // Read sender balance (direct state read)
    let key_low = layout::balance_key(from);
    let key_high = layout::balance_key_high(from);

    let from_balance_low = felt_to_u128(state.get_storage_at(token_address, key_low)?)?;
    let from_balance_high = felt_to_u128(state.get_storage_at(token_address, key_high)?)?;
    let from_balance = u256_from_parts(from_balance_low, from_balance_high);

    if from_balance < amount {
        return Err(ExecutionError::ExecutionFailed(format!(
            "Insufficient balance for fee: have {}, need {}",
            from_balance, amount
        )));
    }

    // Calculate new sender balance
    let new_from_balance = from_balance - amount;
    let (new_from_low, new_from_high) = u256_to_parts(new_from_balance);

    // Read recipient (sequencer) balance
    let to_key_low = layout::balance_key(to);
    let to_key_high = layout::balance_key_high(to);

    let to_balance_low = felt_to_u128(state.get_storage_at(token_address, to_key_low)?)?;
    let to_balance_high = felt_to_u128(state.get_storage_at(token_address, to_key_high)?)?;
    let to_balance = u256_from_parts(to_balance_low, to_balance_high);

    // Calculate new recipient balance
    let new_to_balance = to_balance + amount;
    let (new_to_low, new_to_high) = u256_to_parts(new_to_balance);

    // Add to state diff
    state_diff.storage_updates.entry(token_address).or_default().insert(key_low, Felt::from(new_from_low));

    if new_from_high != from_balance_high {
        state_diff.storage_updates.entry(token_address).or_default().insert(key_high, Felt::from(new_from_high));
    }

    state_diff.storage_updates.entry(token_address).or_default().insert(to_key_low, Felt::from(new_to_low));

    if new_to_high != to_balance_high {
        state_diff.storage_updates.entry(token_address).or_default().insert(to_key_high, Felt::from(new_to_high));
    }

    Ok(())
}

/// Convert a Felt to u128.
fn felt_to_u128(felt: Felt) -> Result<u128, ExecutionError> {
    // Get the bytes and convert
    let bytes = felt.to_bytes_be();
    // Check if value fits in u128 (first 16 bytes should be zero)
    if bytes.iter().take(16).any(|&b| b != 0) {
        return Err(ExecutionError::ExecutionFailed("Value too large for u128".to_string()));
    }
    let mut arr = [0u8; 16];
    arr.copy_from_slice(&bytes[16..32]);
    Ok(u128::from_be_bytes(arr))
}

/// Combine low and high parts into u256 (as u128, assuming high is 0).
fn u256_from_parts(low: u128, _high: u128) -> u128 {
    // For simplicity, we only support values that fit in u128
    // A full implementation would use a proper u256 type
    low
}

/// Split u128 into low and high parts (high is always 0 for u128).
fn u256_to_parts(value: u128) -> (u128, u128) {
    (value, 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_felt_to_u128() {
        let felt = Felt::from(12345u64);
        let result = felt_to_u128(felt).unwrap();
        assert_eq!(result, 12345);
    }

    #[test]
    fn test_u256_parts() {
        let value = 1_000_000u128;
        let (low, high) = u256_to_parts(value);
        assert_eq!(low, 1_000_000);
        assert_eq!(high, 0);

        let reconstructed = u256_from_parts(low, high);
        assert_eq!(reconstructed, value);
    }
}
