use starknet_types_core::felt::Felt;

use crate::contracts::paradex::oracle;
use crate::contracts::paradex::schema::oracle_layout;
use crate::core::state::mock::MockStateReader;
use crate::core::storage::{function_selector, short_string_to_felt, storage_key_for_map, storage_key_with_offset};
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

#[test]
fn test_set_prices_and_funding_snapshot_inserts_and_updates_oracle_state() {
    let mut state = MockStateReader::new();
    let contract = addr(0x206);
    let price_new = short_str("ETH-USD-PERP");
    let price_existing = short_str("BTC-USD-PERP");
    let funding_new = short_str("SOL-USD-PERP");
    let funding_existing = short_str("DOGE-USD-PERP");

    set_storage(&mut state, contract, *oracle_layout::LATEST_UPDATED_TIMESTAMP_BASE, felt(1000));
    set_oracle_latest_tick_data(&mut state, contract, price_existing, price_existing, felt(22), felt(8));
    set_oracle_funding_index(&mut state, contract, funding_existing, felt(33));

    let selector = function_selector("set_prices_and_funding_snapshot");
    let timestamp = felt(1234);
    let calldata = vec![
        felt(77),
        felt(2),
        price_new,
        felt(111),
        felt(8),
        timestamp,
        price_existing,
        felt(222),
        felt(8),
        timestamp,
        felt(2),
        funding_new,
        felt(333),
        felt(8),
        timestamp,
        funding_existing,
        felt(444),
        felt(8),
        timestamp,
    ];

    let result = oracle::execute(&state, contract, selector, &calldata, ContractAddress(Felt::ZERO)).expect("execute");

    assert!(result.call_result.retdata.is_empty());
    let updates = result.state_diff.storage_updates.get(&contract).expect("oracle updates");
    assert_eq!(updates.get(&*oracle_layout::LATEST_UPDATED_TIMESTAMP_BASE).copied(), Some(timestamp));
    assert_eq!(updates.get(&*oracle_layout::LATEST_SNAPSHOT_ID_BASE).copied(), Some(felt(77)));

    let price_new_base = storage_key_for_map("latest_tick_data", price_new);
    assert_eq!(updates.get(&price_new_base).copied(), Some(price_new));
    assert_eq!(updates.get(&storage_key_with_offset(price_new_base, 1)).copied(), Some(felt(111)));
    assert_eq!(updates.get(&storage_key_with_offset(price_new_base, 2)).copied(), Some(felt(8)));

    let price_existing_base = storage_key_for_map("latest_tick_data", price_existing);
    assert_eq!(updates.get(&storage_key_with_offset(price_existing_base, 1)).copied(), Some(felt(222)));
    assert!(!updates.contains_key(&price_existing_base));
    assert!(!updates.contains_key(&storage_key_with_offset(price_existing_base, 2)));

    let funding_new_base = storage_key_for_map("funding_index_data", funding_new);
    assert_eq!(updates.get(&funding_new_base).copied(), Some(funding_new));
    assert_eq!(updates.get(&storage_key_with_offset(funding_new_base, 1)).copied(), Some(felt(333)));
    assert_eq!(updates.get(&storage_key_with_offset(funding_new_base, 2)).copied(), Some(felt(8)));

    let funding_existing_base = storage_key_for_map("funding_index_data", funding_existing);
    assert_eq!(updates.get(&storage_key_with_offset(funding_existing_base, 1)).copied(), Some(felt(444)));
    assert!(!updates.contains_key(&funding_existing_base));
    assert!(!updates.contains_key(&storage_key_with_offset(funding_existing_base, 2)));
}

#[test]
fn test_set_prices_and_funding_snapshot_rejects_stale_timestamp() {
    let mut state = MockStateReader::new();
    let contract = addr(0x207);
    let market = short_str("ETH-USD-PERP");
    set_storage(&mut state, contract, *oracle_layout::LATEST_UPDATED_TIMESTAMP_BASE, felt(1000));

    let selector = function_selector("set_prices_and_funding_snapshot");
    let calldata = vec![felt(77), felt(1), market, felt(111), felt(8), felt(999), felt(0)];

    let err = oracle::execute(&state, contract, selector, &calldata, ContractAddress(Felt::ZERO)).unwrap_err();
    assert!(format!("{err}").contains("TIMESTAMP_TOO_OLD"));
}

#[test]
fn test_set_prices_and_funding_snapshot_rejects_zero_price() {
    let state = MockStateReader::new();
    let contract = addr(0x208);
    let market = short_str("ETH-USD-PERP");

    let selector = function_selector("set_prices_and_funding_snapshot");
    let calldata = vec![felt(77), felt(1), market, felt(0), felt(8), felt(999), felt(0)];

    let err = oracle::execute(&state, contract, selector, &calldata, ContractAddress(Felt::ZERO)).unwrap_err();
    assert!(format!("{err}").contains("price must be positive"));
}
