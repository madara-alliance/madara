use starknet_types_core::felt::Felt;

use crate::contracts::paradex::assets_manager;
use crate::contracts::paradex_codegen::assets_manager_layout;
use crate::state::mock::MockStateReader;
use crate::storage::function_selector;

use super::super::fixtures::{addr, felt, set_spot_asset_substorage, short_str};

#[test]
fn test_assets_manager_supports_selector_known() {
    let selector_kind = function_selector("get_asset_kind");
    let selector_base = function_selector("get_base_token_asset");
    assert!(assets_manager::supports_selector(selector_kind));
    assert!(assets_manager::supports_selector(selector_base));
}

#[test]
fn test_assets_manager_supports_selector_unknown() {
    let selector = Felt::from(999u64);
    assert!(!assets_manager::supports_selector(selector));
}

#[test]
fn test_assets_manager_spot_asset_layout_fallbacks() {
    let mut state = MockStateReader::new();
    let contract = addr(0x105);
    let market = felt(0x205);
    let token_addr = addr(0x99);
    let token_name = short_str("WETH");

    set_spot_asset_substorage(
        &mut state,
        contract,
        *assets_manager_layout::SPOT_BASE,
        market,
        token_addr,
        token_name,
        felt(0xabc),
    );

    let mut ctx = crate::ExecutionContext::new();
    let named = assets_manager::get_base_token_asset(&mut ctx, &state, contract, market).expect("base token");
    assert_eq!(named.token_address, token_addr);
    assert_eq!(named.token_name, token_name);
}
