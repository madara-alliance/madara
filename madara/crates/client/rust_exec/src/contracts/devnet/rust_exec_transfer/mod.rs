//! Devnet-only Rust Exec E2E fixture. This is not production contract support.

use starknet_types_core::felt::Felt;

use crate::contracts::paradex::supported::{supported_contract, SupportedContract};
use crate::contracts::ExecutionError;
use crate::core::context::ExecutionContext;
use crate::core::state::StateReader;
use crate::core::storage::{event_selector, function_selector, storage_key_for_variable};
use crate::core::types::{ContractAddress, ExecutionResult};

pub const NAME: &str = "RustExecTransfer";

fn metadata() -> &'static SupportedContract {
    supported_contract(NAME).expect("RustExecTransfer must be present in supported_contracts.json")
}

pub fn class_hash() -> Felt {
    metadata().class_hash
}

pub fn supports_class_hash(candidate: Felt) -> bool {
    candidate == class_hash()
}

pub fn supports_selector(selector: Felt) -> bool {
    metadata().supported_functions.iter().any(|function| function.selector == selector)
}

pub fn get_function_name(selector: Felt) -> Option<String> {
    metadata()
        .supported_functions
        .iter()
        .find(|function| function.selector == selector)
        .map(|function| function.name.clone())
}

pub fn execute<S: StateReader>(
    state: &S,
    contract: ContractAddress,
    selector: Felt,
    calldata: &[Felt],
    caller: ContractAddress,
) -> Result<ExecutionResult, ExecutionError> {
    if !supports_selector(selector) {
        return Err(ExecutionError::UnknownSelector(selector));
    }
    if calldata.len() != 2 {
        return Err(ExecutionError::ExecutionFailed("transfer takes 2 arguments (recipient, amount)".to_string()));
    }

    let recipient = ContractAddress(calldata[0]);
    let amount = calldata[1];
    // Devnet E2E fixture: force the real comparator mismatch path with one isolated storage difference.
    let stored_amount =
        if selector == function_selector("transfer_with_comparator_mismatch") { amount + Felt::ONE } else { amount };
    let transfer_count_key = storage_key_for_variable("transfer_count");
    let mut ctx = ExecutionContext::new();
    let transfer_count = ctx.storage_read(state, contract, transfer_count_key)?;

    ctx.storage_write(contract, storage_key_for_variable("last_sender"), caller.0);
    ctx.storage_write(contract, storage_key_for_variable("last_recipient"), recipient.0);
    ctx.storage_write(contract, storage_key_for_variable("last_amount"), stored_amount);
    ctx.storage_write(contract, transfer_count_key, transfer_count + Felt::ONE);
    ctx.emit_event(vec![event_selector("Transfer"), caller.0, recipient.0], vec![amount]);
    ctx.set_retdata(vec![Felt::ONE]);

    Ok(ctx.build_result())
}

#[cfg(test)]
mod tests {
    use starknet_core::types::contract::SierraClass;

    use super::*;
    use crate::core::state::mock::MockStateReader;
    use crate::core::storage::function_selector;

    #[test]
    fn manifest_hash_matches_cairo_contract() {
        let class: SierraClass =
            serde_json::from_slice(m_cairo_test_contracts::RUST_EXEC_TRANSFER_SIERRA).expect("valid Sierra artifact");
        assert_eq!(class.flatten().expect("flatten Sierra class").class_hash(), class_hash());
    }

    #[test]
    fn transfer_records_state_and_event() {
        let contract = ContractAddress(Felt::from_hex_unchecked("0x100"));
        let caller = ContractAddress(Felt::from_hex_unchecked("0x200"));
        let recipient = ContractAddress(Felt::from_hex_unchecked("0x300"));
        let amount = Felt::from(42u64);
        let count_key = storage_key_for_variable("transfer_count");
        let mut state = MockStateReader::new();
        state.set_storage(contract, count_key, Felt::from(7u64));

        let result = execute(&state, contract, function_selector("transfer"), &[recipient.0, amount], caller)
            .expect("transfer should execute");
        let updates = &result.state_diff.storage_updates[&contract];

        assert_eq!(updates[&storage_key_for_variable("last_sender")], caller.0);
        assert_eq!(updates[&storage_key_for_variable("last_recipient")], recipient.0);
        assert_eq!(updates[&storage_key_for_variable("last_amount")], amount);
        assert_eq!(updates[&count_key], Felt::from(8u64));
        assert_eq!(result.call_result.retdata, vec![Felt::ONE]);
        assert_eq!(result.call_result.events.len(), 1);
        assert_eq!(result.call_result.events[0].keys, vec![event_selector("Transfer"), caller.0, recipient.0]);
        assert_eq!(result.call_result.events[0].data, vec![amount]);
    }

    #[test]
    fn mismatch_fixture_changes_only_stored_amount() {
        let contract = ContractAddress(Felt::from_hex_unchecked("0x100"));
        let caller = ContractAddress(Felt::from_hex_unchecked("0x200"));
        let recipient = ContractAddress(Felt::from_hex_unchecked("0x300"));
        let amount = Felt::from(42u64);
        let result = execute(
            &MockStateReader::new(),
            contract,
            function_selector("transfer_with_comparator_mismatch"),
            &[recipient.0, amount],
            caller,
        )
        .expect("mismatch fixture should execute in Rust");
        let updates = &result.state_diff.storage_updates[&contract];

        assert_eq!(updates[&storage_key_for_variable("last_sender")], caller.0);
        assert_eq!(updates[&storage_key_for_variable("last_recipient")], recipient.0);
        assert_eq!(updates[&storage_key_for_variable("last_amount")], amount + Felt::ONE);
        assert_eq!(updates[&storage_key_for_variable("transfer_count")], Felt::ONE);
        assert_eq!(result.call_result.events[0].data, vec![amount]);
    }

    #[test]
    fn transfer_rejects_invalid_calldata() {
        let state = MockStateReader::new();
        let result = execute(
            &state,
            ContractAddress(Felt::ONE),
            function_selector("transfer"),
            &[Felt::from(2u64)],
            ContractAddress(Felt::from(3u64)),
        );

        assert!(matches!(
            result,
            Err(ExecutionError::ExecutionFailed(message)) if message.contains("takes 2 arguments")
        ));
    }
}
