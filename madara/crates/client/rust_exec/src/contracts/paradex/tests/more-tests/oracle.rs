use starknet_types_core::felt::Felt;

use crate::contracts::paradex::oracle;
use crate::contracts::paradex_codegen::oracle_layout;
use crate::state::mock::MockStateReader;
use crate::storage::function_selector;
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
