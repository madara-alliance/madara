use starknet_types_core::felt::Felt;

use crate::contracts::paradex::oracle;
use crate::contracts::paradex_codegen::oracle_layout;
use crate::state::mock::MockStateReader;
use crate::storage::{function_selector, storage_key_for_map, storage_key_with_offset};
use crate::types::ContractAddress;

use super::super::fixtures::{
    addr, felt, set_oracle_funding_index, set_oracle_latest_tick_data, set_storage, short_str,
};

#[test]
fn test_oracle_supports_selector_known() {
    let selectors = [
        function_selector("get_value"),
        function_selector("get_values_with_funding_indices"),
        function_selector("get_funding_index"),
        function_selector("get_latest_snapshot_id"),
        function_selector("decimals"),
        function_selector("get_version"),
    ];
    for selector in selectors {
        assert!(oracle::supports_selector(selector));
    }
}

#[test]
fn test_oracle_supports_selector_unknown() {
    let selector = Felt::from(999u64);
    assert!(!oracle::supports_selector(selector));
}

#[test]
fn test_oracle_get_latest_snapshot_id() {
    let mut state = MockStateReader::new();
    let contract = addr(0x203);
    set_storage(&mut state, contract, *oracle_layout::LATEST_SNAPSHOT_ID_BASE, felt(0x55));

    let selector = function_selector("get_latest_snapshot_id");
    let result = oracle::execute(&state, contract, selector, &[], ContractAddress(Felt::ZERO)).expect("execute");
    assert_eq!(result.call_result.retdata, vec![felt(0x55)]);
}

#[test]
fn test_oracle_decode_felt_array_underflow() {
    let state = MockStateReader::new();
    let contract = addr(0x206);

    let selector = function_selector("get_values_with_funding_indices");
    let err =
        oracle::execute(&state, contract, selector, &[felt(2), felt(0x1)], ContractAddress(Felt::ZERO)).unwrap_err();
    assert!(format!("{err}").contains("array underflow"));
}

#[test]
fn test_oracle_latest_tick_data_base_cache_hit() {
    let mut state = MockStateReader::new();
    let contract = addr(0x207);
    let market = felt(0x88);
    set_oracle_latest_tick_data(&mut state, contract, market, felt(0x1), felt(0x2), felt(8));
    set_storage(&mut state, contract, *oracle_layout::LATEST_UPDATED_TIMESTAMP_BASE, felt(1));

    let mut ctx = crate::ExecutionContext::new();
    let first = oracle::read_tick_data(&mut ctx, &state, contract, market).expect("read tick");
    let second = oracle::read_tick_data(&mut ctx, &state, contract, market).expect("read tick");
    assert_eq!(first, second);
}

#[test]
fn test_oracle_funding_index_base_cache_hit() {
    let mut state = MockStateReader::new();
    let contract = addr(0x208);
    let market = felt(0x99);
    set_oracle_funding_index(&mut state, contract, market, felt(0x123));

    let mut ctx = crate::ExecutionContext::new();
    let first = oracle::read_funding_index(&mut ctx, &state, contract, market).expect("read index");
    let second = oracle::read_funding_index(&mut ctx, &state, contract, market).expect("read index");
    assert_eq!(first, second);
}

#[test]
fn test_oracle_settlement_token_price_reads_latest_tick_data() {
    let mut state = MockStateReader::new();
    let contract = addr(0x209);
    let usdc = short_str("USDC");
    set_oracle_latest_tick_data(&mut state, contract, usdc, usdc, felt(0x777), felt(8));

    let mut ctx = crate::ExecutionContext::new();
    let price = oracle::read_settlement_token_price(&mut ctx, &state, contract, usdc).expect("price");
    assert_eq!(price, felt(0x777));
}

#[test]
fn test_oracle_set_prices_and_funding_snapshot_updates_state_diff() {
    let mut state = MockStateReader::new();
    let contract = addr(0x20a);
    let snapshot_id = felt(0x55);
    let price_asset = felt(0x88);
    let funding_asset = felt(0x99);
    let timestamp = felt(0x64);

    set_storage(&mut state, contract, *oracle_layout::LATEST_UPDATED_TIMESTAMP_BASE, felt(0x63));

    let selector = function_selector("set_prices_and_funding_snapshot");
    let calldata = vec![
        snapshot_id,
        felt(1),
        price_asset,
        felt(0x777),
        felt(8),
        timestamp,
        felt(1),
        funding_asset,
        felt(0x111),
        felt(8),
        timestamp,
    ];

    let result = oracle::execute(&state, contract, selector, &calldata, ContractAddress(Felt::ZERO)).expect("execute");
    let writes = result.state_diff.storage_updates.get(&contract).expect("oracle call should emit storage writes");

    assert_eq!(writes.get(&*oracle_layout::LATEST_UPDATED_TIMESTAMP_BASE), Some(&timestamp));
    assert_eq!(writes.get(&*oracle_layout::LATEST_SNAPSHOT_ID_BASE), Some(&snapshot_id));

    let latest_tick_base = storage_key_for_map("latest_tick_data", price_asset);
    assert_eq!(writes.get(&latest_tick_base), Some(&price_asset));
    assert_eq!(writes.get(&storage_key_with_offset(latest_tick_base, 1)), Some(&felt(0x777)));
    assert_eq!(writes.get(&storage_key_with_offset(latest_tick_base, 2)), Some(&felt(8)));

    let funding_index_base = storage_key_for_map("funding_index_data", funding_asset);
    assert_eq!(writes.get(&funding_index_base), Some(&funding_asset));
    assert_eq!(writes.get(&storage_key_with_offset(funding_index_base, 1)), Some(&felt(0x111)));
    assert_eq!(writes.get(&storage_key_with_offset(funding_index_base, 2)), Some(&felt(8)));
}

#[test]
fn test_oracle_set_prices_and_funding_snapshot_uses_struct_counts() {
    let mut state = MockStateReader::new();
    let contract = addr(0x20b);
    let snapshot_id = felt(0x77);
    let price_asset_1 = felt(0x101);
    let price_asset_2 = felt(0x102);
    let funding_asset = felt(0x201);
    let timestamp = felt(0x65);

    set_storage(&mut state, contract, *oracle_layout::LATEST_UPDATED_TIMESTAMP_BASE, felt(0x64));

    let selector = function_selector("set_prices_and_funding_snapshot");
    let calldata = vec![
        snapshot_id,
        felt(2),
        price_asset_1,
        felt(0x111),
        felt(8),
        timestamp,
        price_asset_2,
        felt(0x222),
        felt(8),
        timestamp,
        felt(1),
        funding_asset,
        felt(0x333),
        felt(8),
        timestamp,
    ];

    let result = oracle::execute(&state, contract, selector, &calldata, ContractAddress(Felt::ZERO)).expect("execute");
    let writes = result.state_diff.storage_updates.get(&contract).expect("oracle call should emit storage writes");

    assert_eq!(writes.get(&*oracle_layout::LATEST_UPDATED_TIMESTAMP_BASE), Some(&timestamp));
    assert_eq!(writes.get(&*oracle_layout::LATEST_SNAPSHOT_ID_BASE), Some(&snapshot_id));

    let latest_tick_base_1 = storage_key_for_map("latest_tick_data", price_asset_1);
    assert_eq!(writes.get(&latest_tick_base_1), Some(&price_asset_1));
    assert_eq!(writes.get(&storage_key_with_offset(latest_tick_base_1, 1)), Some(&felt(0x111)));

    let latest_tick_base_2 = storage_key_for_map("latest_tick_data", price_asset_2);
    assert_eq!(writes.get(&latest_tick_base_2), Some(&price_asset_2));
    assert_eq!(writes.get(&storage_key_with_offset(latest_tick_base_2, 1)), Some(&felt(0x222)));

    let funding_index_base = storage_key_for_map("funding_index_data", funding_asset);
    assert_eq!(writes.get(&funding_index_base), Some(&funding_asset));
    assert_eq!(writes.get(&storage_key_with_offset(funding_index_base, 1)), Some(&felt(0x333)));
}
