//! Function implementations for Random100Hashes contract.

use starknet_crypto::pedersen_hash;
use starknet_types_core::felt::Felt;

use crate::context::ExecutionContext;
use crate::contracts::ExecutionError;
use crate::state::StateReader;
use crate::types::ContractAddress;

use super::layout::LAST_HASH_RESULT_KEY;

/// Execute the compute_100_hashes() function.
///
/// This function matches the Cairo implementation:
/// 1. Takes a seed value from calldata
/// 2. Performs 100 Pedersen hash computations in a chain:
///    h0 = pedersen(seed, 0)
///    h1 = pedersen(h0, 1)
///    ...
///    h99 = pedersen(h98, 99)
/// 3. Stores the final result in storage
/// 4. Returns the final hash result
pub fn execute_compute_100_hashes<S: StateReader>(
    _state: &S,
    contract: ContractAddress,
    seed: Felt,
    ctx: &mut ExecutionContext,
) -> Result<(), ExecutionError> {
    let mut current_hash = seed;

    // Perform 100 Pedersen hash computations
    for i in 0u64..100 {
        current_hash = pedersen_hash(&current_hash, &Felt::from(i));
    }

    // Store the result
    ctx.storage_write(contract, *LAST_HASH_RESULT_KEY, current_hash);

    // Return the final hash result
    ctx.set_retdata(vec![current_hash]);

    Ok(())
}

/// Execute the get_last_result() function (read-only).
///
/// Returns the last computed hash result.
pub fn execute_get_last_result<S: StateReader>(
    state: &S,
    contract: ContractAddress,
    ctx: &mut ExecutionContext,
) -> Result<(), ExecutionError> {
    let value = ctx.storage_read(state, contract, *LAST_HASH_RESULT_KEY)?;
    ctx.set_retdata(vec![value]);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::mock::MockStateReader;

    #[test]
    fn test_compute_100_hashes_deterministic() {
        let state = MockStateReader::new();
        let contract = ContractAddress(Felt::from(1u64));
        let seed = Felt::from(12345u64);

        let mut ctx1 = ExecutionContext::new();
        execute_compute_100_hashes(&state, contract, seed, &mut ctx1).unwrap();
        let result1 = ctx1.build_result();

        let mut ctx2 = ExecutionContext::new();
        execute_compute_100_hashes(&state, contract, seed, &mut ctx2).unwrap();
        let result2 = ctx2.build_result();

        // Same seed should produce same result
        assert_eq!(result1.call_result.retdata, result2.call_result.retdata);
    }

    #[test]
    fn test_compute_100_hashes_different_seeds() {
        let state = MockStateReader::new();
        let contract = ContractAddress(Felt::from(1u64));

        let mut ctx1 = ExecutionContext::new();
        execute_compute_100_hashes(&state, contract, Felt::from(1u64), &mut ctx1).unwrap();
        let result1 = ctx1.build_result();

        let mut ctx2 = ExecutionContext::new();
        execute_compute_100_hashes(&state, contract, Felt::from(2u64), &mut ctx2).unwrap();
        let result2 = ctx2.build_result();

        // Different seeds should produce different results
        assert_ne!(result1.call_result.retdata, result2.call_result.retdata);
    }

    #[test]
    fn test_compute_100_hashes_stores_result() {
        let state = MockStateReader::new();
        let contract = ContractAddress(Felt::from(1u64));
        let seed = Felt::from(42u64);

        let mut ctx = ExecutionContext::new();
        execute_compute_100_hashes(&state, contract, seed, &mut ctx).unwrap();
        let result = ctx.build_result();

        // Should have stored the result
        let stored_value = result
            .state_diff
            .storage_updates
            .get(&contract)
            .unwrap()
            .get(&*LAST_HASH_RESULT_KEY)
            .unwrap();

        // Stored value should match return value
        assert_eq!(*stored_value, result.call_result.retdata[0]);
    }

    #[test]
    fn test_get_last_result() {
        let mut state = MockStateReader::new();
        let contract = ContractAddress(Felt::from(1u64));
        let expected_value = Felt::from(999u64);
        state.set_storage(contract, *LAST_HASH_RESULT_KEY, expected_value);

        let mut ctx = ExecutionContext::new();
        execute_get_last_result(&state, contract, &mut ctx).unwrap();

        let result = ctx.build_result();
        assert_eq!(result.call_result.retdata, vec![expected_value]);
    }
}
