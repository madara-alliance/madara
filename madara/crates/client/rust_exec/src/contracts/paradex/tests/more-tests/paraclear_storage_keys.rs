use super::super::fixtures::{addr, felt, set_storage};
use crate::contracts::paradex::paraclear;
use crate::contracts::paradex::schema::paraclear_layout;
use crate::core::state::mock::MockStateReader;
use crate::core::storage::{
    storage_key_for_map, storage_key_for_map2, storage_key_for_substorage_map2_poseidon,
    storage_key_for_substorage_map_poseidon, storage_key_for_substorage_var_poseidon, storage_key_for_variable,
};
use crate::core::types::ContractAddress;
use starknet_types_core::felt::Felt;

fn felt_hex(value: &str) -> Felt {
    Felt::from_hex_unchecked(value)
}

fn addr_hex(value: &str) -> ContractAddress {
    ContractAddress(felt_hex(value))
}

#[test]
fn test_perp_map_key_legacy_pedersen() {
    let key = felt(0x11);
    let expected = storage_key_for_map("Paraclear_perpetual_asset", key);
    let got = paraclear::perp_map_key_for_test(0, "Paraclear_perpetual_asset", key);
    assert_eq!(got, expected);
}

#[test]
fn test_perp_map2_key_legacy_pedersen() {
    let key1 = felt(0x22);
    let key2 = felt(0x33);
    let expected = storage_key_for_map2("Paraclear_perpetual_asset_balance", key1, key2);
    let got = paraclear::perp_map2_key_for_test(0, "Paraclear_perpetual_asset_balance", key1, key2);
    assert_eq!(got, expected);
}

#[test]
fn test_perp_var_key_legacy_pedersen() {
    let expected = storage_key_for_variable("perpetual_futures_mmf_factor");
    let got = paraclear::perp_var_key_for_test(0, "perpetual_futures_mmf_factor");
    assert_eq!(got, expected);
}

#[test]
fn test_perp_map_key_substorage() {
    let base = *paraclear_layout::PERPETUAL_FUTURE_BASE;
    let key = felt(0x44);
    let expected = storage_key_for_substorage_map_poseidon(base, "perpetual_future_asset", key);
    let got = paraclear::perp_map_key_for_test(2, "perpetual_future_asset", key);
    assert_eq!(got, expected);
}

#[test]
fn test_perp_map2_key_substorage() {
    let base = *paraclear_layout::PERPETUAL_FUTURE_BASE;
    let key1 = felt(0x55);
    let key2 = felt(0x66);
    let expected = storage_key_for_substorage_map2_poseidon(base, "perpetual_future_position", key1, key2);
    let got = paraclear::perp_map2_key_for_test(2, "perpetual_future_position", key1, key2);
    assert_eq!(got, expected);
}

#[test]
fn test_perp_var_key_substorage() {
    let base = *paraclear_layout::PERPETUAL_FUTURE_BASE;
    let expected = storage_key_for_substorage_var_poseidon(base, "perpetual_futures_mmf_factor");
    let got = paraclear::perp_var_key_for_test(2, "perpetual_futures_mmf_factor");
    assert_eq!(got, expected);
}

#[test]
fn test_resolve_perp_storage_mode_legacy_pedersen() {
    let mut state = MockStateReader::new();
    let contract = addr(0x300);
    let market = felt(0x77);
    let key = storage_key_for_map("Paraclear_perpetual_asset", market);
    set_storage(&mut state, contract, key, market);

    let mut ctx = crate::ExecutionContext::new();
    let mode = paraclear::resolve_perp_storage_mode_for_test(&mut ctx, &state, contract, market);
    assert_eq!(mode, 0);
}

#[test]
fn test_resolve_perp_balance_base() {
    let state = MockStateReader::new();
    let contract = addr(0x301);
    let account = addr(0x302);
    let market = felt(0x88);
    let mut ctx = crate::ExecutionContext::new();

    let first =
        paraclear::resolve_perp_balance_base_for_test(&state, &mut ctx, contract, account, market).expect("base");
    let second =
        paraclear::resolve_perp_balance_base_for_test(&state, &mut ctx, contract, account, market).expect("base");
    assert_eq!(first, second);
}

#[test]
fn test_mainnet_perp_balance_base_matches_cairo_map2_key() {
    let state = MockStateReader::new();
    let contract = addr(0x301);
    let maker = addr_hex("0x7158925a5820b47914ab2273d6c5b245efce5ccb51eb03649914c326f4a4da7");
    let taker = addr_hex("0x7a11fc1604320402a07cb00a1e22150ebc81f7ef8b120c2bc9995ba6f3ef18a");
    let market = felt_hex("0x4254432d5553442d3741554732362d36333030302d50");
    let mut ctx = crate::ExecutionContext::new();

    for account in [maker, taker] {
        let got =
            paraclear::resolve_perp_balance_base_for_test(&state, &mut ctx, contract, account, market).expect("base");
        let expected = storage_key_for_map2("Paraclear_perpetual_asset_balance", account.0, market);
        assert_eq!(got, expected);
    }
}

#[test]
fn test_mainnet_token_balance_base_matches_cairo_map2_key() {
    let state = MockStateReader::new();
    let contract = addr(0x301);
    let account = addr_hex("0x7158925a5820b47914ab2273d6c5b245efce5ccb51eb03649914c326f4a4da7");
    let token = addr_hex("0x47adc7dee88eec362d71a52c25d40559a921434b2d90e75b6a4a6e4e9fb9ab1");
    let mut ctx = crate::ExecutionContext::new();

    let got = paraclear::resolve_token_balance_base_for_test(&state, &mut ctx, contract, account, token).expect("base");
    let expected = storage_key_for_map2("Paraclear_token_asset_balance", account.0, token.0);
    assert_eq!(got, expected);
}
