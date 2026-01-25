//! Function implementations for SimpleCounter contract.

use starknet_types_core::felt::Felt;

use crate::context::ExecutionContext;
use crate::contracts::ExecutionError;
use crate::state::StateReader;
use crate::storage::event_selector;
use crate::types::ContractAddress;

use super::layout::X_KEY;

/// Execute the increment() function.
///
/// This function:
/// 1. Reads current value of `x`
/// 2. Increments it by 2
/// 3. Writes new value back
/// 4. Emits an event with old and new values
/// 5. Returns the new value
pub fn execute_increment<S: StateReader>(
    state: &S,
    contract: ContractAddress,
    ctx: &mut ExecutionContext,
) -> Result<(), ExecutionError> {
    // 1. Read current value of x
    let old_value = ctx.storage_read(state, contract, *X_KEY)?;

    // 2. Increment by 2
    let new_value = old_value + Felt::TWO;

    // 3. Write new value
    ctx.storage_write(contract, *X_KEY, new_value);

    // 4. Emit event: ValueChanged(old_value, new_value)
    let event_key = event_selector("ValueChanged");
    ctx.emit_event(vec![event_key], vec![old_value, new_value]);

    // 5. Set return data (new value)
    ctx.set_retdata(vec![new_value]);

    Ok(())
}

#[cfg(test)]
mod tests {dd
    use super::*;
    use crate::state::mock::MockStateReader;

    #[test]
    fn test_increment_from_zero() {
        let state = MockStateReader::new();
        let contract = ContractAddress(Felt::from(1u64));
        let mut ctx = ExecutionContext::new();

        execute_increment(&state, contract, &mut ctx).unwrap();

        let result = ctx.build_result();
        assert_eq!(result.call_result.retdata, vec![Felt::TWO]);
        assert_eq!(result.call_result.events.len(), 1);
        assert_eq!(result.call_result.events[0].data, vec![Felt::ZERO, Felt::TWO]);
    }

    #[test]
    fn test_increment_from_existing_value() {
        let mut state = MockStateReader::new();
        let contract = ContractAddress(Felt::from(1u64));
        state.set_storage(contract, *X_KEY, Felt::from(10u64));

        let mut ctx = ExecutionContext::new();

        execute_increment(&state, contract, &mut ctx).unwrap();

        let result = ctx.build_result();
        assert_eq!(result.call_result.retdata, vec![Felt::from(12u64)]);
        assert_eq!(result.call_result.events[0].data, vec![Felt::from(10u64), Felt::from(12u64)]);
    }
}
