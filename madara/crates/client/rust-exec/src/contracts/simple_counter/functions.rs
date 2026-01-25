//! Function implementations for SimpleCounter contract.

use starknet_types_core::felt::Felt;

use crate::context::ExecutionContext;
use crate::contracts::ExecutionError;
use crate::state::StateReader;
use crate::types::ContractAddress;

use super::layout::COUNTER_KEY;

/// Execute the increment() function.
///
/// This function matches the Cairo implementation:
/// 1. Reads current value of `counter`
/// 2. Increments it by 1
/// 3. Writes new value back
/// 4. No event emitted
/// 5. No return value
pub fn execute_increment<S: StateReader>(
    state: &S,
    contract: ContractAddress,
    ctx: &mut ExecutionContext,
) -> Result<(), ExecutionError> {
    // 1. Read current value of counter
    let current = ctx.storage_read(state, contract, *COUNTER_KEY)?;

    // 2. Increment by 1
    let new_value = current + Felt::ONE;

    // 3. Write new value
    ctx.storage_write(contract, *COUNTER_KEY, new_value);

    // Cairo increment() has no return value and emits no events

    Ok(())
}

/// Execute the get_counter() function (read-only).
///
/// Returns the current counter value.
pub fn execute_get_counter<S: StateReader>(
    state: &S,
    contract: ContractAddress,
    ctx: &mut ExecutionContext,
) -> Result<(), ExecutionError> {
    let value = ctx.storage_read(state, contract, *COUNTER_KEY)?;
    ctx.set_retdata(vec![value]);
    Ok(())
}

/// Execute the decrement() function.
///
/// Decrements the counter by 1.
pub fn execute_decrement<S: StateReader>(
    state: &S,
    contract: ContractAddress,
    ctx: &mut ExecutionContext,
) -> Result<(), ExecutionError> {
    let current = ctx.storage_read(state, contract, *COUNTER_KEY)?;
    let new_value = current - Felt::ONE;
    ctx.storage_write(contract, *COUNTER_KEY, new_value);
    Ok(())
}

/// Execute the set_counter(value) function.
///
/// Sets the counter to the specified value.
pub fn execute_set_counter<S: StateReader>(
    _state: &S,
    contract: ContractAddress,
    ctx: &mut ExecutionContext,
    value: Felt,
) -> Result<(), ExecutionError> {
    ctx.storage_write(contract, *COUNTER_KEY, value);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::mock::MockStateReader;

    #[test]
    fn test_increment_from_zero() {
        let state = MockStateReader::new();
        let contract = ContractAddress(Felt::from(1u64));
        let mut ctx = ExecutionContext::new();

        execute_increment(&state, contract, &mut ctx).unwrap();

        let result = ctx.build_result();
        // Cairo increment() has no return value and no events
        assert!(result.call_result.retdata.is_empty());
        assert!(result.call_result.events.is_empty());
        // Check storage was updated: 0 + 1 = 1
        assert_eq!(
            result.state_diff.storage_updates.get(&contract).unwrap().get(&*COUNTER_KEY).unwrap(),
            &Felt::ONE
        );
    }

    #[test]
    fn test_increment_from_existing_value() {
        let mut state = MockStateReader::new();
        let contract = ContractAddress(Felt::from(1u64));
        state.set_storage(contract, *COUNTER_KEY, Felt::from(10u64));

        let mut ctx = ExecutionContext::new();

        execute_increment(&state, contract, &mut ctx).unwrap();

        let result = ctx.build_result();
        // Cairo increment() has no return value and no events
        assert!(result.call_result.retdata.is_empty());
        assert!(result.call_result.events.is_empty());
        // Check storage was updated: 10 + 1 = 11
        assert_eq!(
            result.state_diff.storage_updates.get(&contract).unwrap().get(&*COUNTER_KEY).unwrap(),
            &Felt::from(11u64)
        );
    }

    #[test]
    fn test_get_counter() {
        let mut state = MockStateReader::new();
        let contract = ContractAddress(Felt::from(1u64));
        state.set_storage(contract, *COUNTER_KEY, Felt::from(42u64));

        let mut ctx = ExecutionContext::new();

        execute_get_counter(&state, contract, &mut ctx).unwrap();

        let result = ctx.build_result();
        assert_eq!(result.call_result.retdata, vec![Felt::from(42u64)]);
    }

    #[test]
    fn test_decrement() {
        let mut state = MockStateReader::new();
        let contract = ContractAddress(Felt::from(1u64));
        state.set_storage(contract, *COUNTER_KEY, Felt::from(10u64));

        let mut ctx = ExecutionContext::new();

        execute_decrement(&state, contract, &mut ctx).unwrap();

        let result = ctx.build_result();
        assert!(result.call_result.retdata.is_empty());
        assert_eq!(
            result.state_diff.storage_updates.get(&contract).unwrap().get(&*COUNTER_KEY).unwrap(),
            &Felt::from(9u64)
        );
    }

    #[test]
    fn test_set_counter() {
        let state = MockStateReader::new();
        let contract = ContractAddress(Felt::from(1u64));

        let mut ctx = ExecutionContext::new();

        execute_set_counter(&state, contract, &mut ctx, Felt::from(100u64)).unwrap();

        let result = ctx.build_result();
        assert!(result.call_result.retdata.is_empty());
        assert_eq!(
            result.state_diff.storage_updates.get(&contract).unwrap().get(&*COUNTER_KEY).unwrap(),
            &Felt::from(100u64)
        );
    }
}
