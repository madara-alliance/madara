use starknet_types_core::felt::Felt;

use crate::contracts::paradex::oracle;
use crate::contracts::paradex::schema::oracle_layout;
use crate::core::state::mock::MockStateReader;
use crate::core::storage::{function_selector, short_string_to_felt};
use crate::core::types::ContractAddress;

use super::fixtures::{addr, felt, set_oracle_funding_index, set_oracle_latest_tick_data, set_storage, short_str};

#[test]
fn test_get_value() {
    let mut state = MockStateReader::new();
    let contract = addr(0x200);
    let market = felt(0x1);
    set_oracle_latest_tick_data(&mut state, contract, market, felt(0x55), felt(0x66), felt(8));
    set_storage(&mut state, contract, *oracle_layout::LATEST_UPDATED_TIMESTAMP_BASE, felt(999));

    let selector = function_selector("get_value");
    let result = oracle::execute(&state, contract, selector, &[market], ContractAddress(Felt::ZERO)).expect("execute");
    assert_eq!(result.call_result.retdata, vec![felt(0x55), felt(0x66), felt(8), felt(999)]);
}

#[test]
fn test_get_values_with_funding_indices() {
    let mut state = MockStateReader::new();
    let contract = addr(0x201);
    let market_a = felt(0x10);
    let market_b = felt(0x20);

    set_oracle_latest_tick_data(&mut state, contract, market_a, felt(0x1), felt(0x100), felt(8));
    set_oracle_latest_tick_data(&mut state, contract, market_b, felt(0x2), felt(0x200), felt(8));
    set_oracle_funding_index(&mut state, contract, market_a, felt(0x111));
    set_oracle_funding_index(&mut state, contract, market_b, felt(0x222));

    // settlement token price is read from latest_tick_data("USDC")
    let usdc = short_str("USDC");
    set_oracle_latest_tick_data(&mut state, contract, usdc, usdc, felt(0x999), felt(8));

    let selector = function_selector("get_values_with_funding_indices");
    let calldata = vec![felt(2), market_a, market_b];
    let result = oracle::execute(&state, contract, selector, &calldata, ContractAddress(Felt::ZERO)).expect("execute");

    assert_eq!(
        result.call_result.retdata,
        vec![felt(2), felt(0x100), felt(0x200), felt(2), felt(0x111), felt(0x222), felt(0x999),]
    );
}

#[test]
fn test_funding_index() {
    let mut state = MockStateReader::new();
    let contract = addr(0x202);
    let market = felt(0x33);
    set_oracle_funding_index(&mut state, contract, market, felt(0xabc));

    let selector = function_selector("get_funding_index");
    let result = oracle::execute(&state, contract, selector, &[market], ContractAddress(Felt::ZERO)).expect("execute");
    assert_eq!(result.call_result.retdata, vec![felt(0xabc)]);
}

#[test]
fn test_decimals() {
    let state = MockStateReader::new();
    let contract = addr(0x204);

    let selector = function_selector("decimals");
    let result = oracle::execute(&state, contract, selector, &[], ContractAddress(Felt::ZERO)).expect("execute");
    assert_eq!(result.call_result.retdata, vec![felt(8)]);
}

#[test]
fn test_version() {
    let state = MockStateReader::new();
    let contract = addr(0x205);

    let selector = function_selector("get_version");
    let result = oracle::execute(&state, contract, selector, &[], ContractAddress(Felt::ZERO)).expect("execute");
    assert_eq!(result.call_result.retdata, vec![short_string_to_felt("1.0.9")]);
}
