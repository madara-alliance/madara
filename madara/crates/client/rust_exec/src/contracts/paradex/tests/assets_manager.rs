use crate::contracts::paradex::assets_manager;
use crate::core::state::mock::MockStateReader;
use crate::core::storage::{storage_key_for_map, storage_key_with_offset};

use super::fixtures::{
    addr, felt, set_future_asset_direct, set_option_asset_direct, set_spot_asset_direct, set_storage, short_str,
};

#[test]
fn test_get_asset_kind_future() {
    let mut state = MockStateReader::new();
    let contract = addr(0x100);
    let market = felt(0x200);
    set_future_asset_direct(
        &mut state,
        contract,
        market,
        felt(0x1),
        felt(0x2),
        felt(0x3),
        felt(0x4),
        felt(0x5),
        felt(0x6),
        felt(0x7),
    );

    let mut ctx = crate::ExecutionContext::new();
    let kind = assets_manager::get_asset_kind(&mut ctx, &state, contract, market).expect("kind");
    assert_eq!(kind, assets_manager::asset_kind_future());
}

#[test]
fn test_get_asset_kind_option() {
    let mut state = MockStateReader::new();
    let contract = addr(0x101);
    let market = felt(0x201);
    set_option_asset_direct(&mut state, contract, market, felt(0x11), felt(0x12), felt(0x13), felt(0x14), felt(0x15));

    let mut ctx = crate::ExecutionContext::new();
    let kind = assets_manager::get_asset_kind(&mut ctx, &state, contract, market).expect("kind");
    assert_eq!(kind, assets_manager::asset_kind_option());
}

#[test]
fn test_get_asset_kind_spot() {
    let mut state = MockStateReader::new();
    let contract = addr(0x102);
    let market = felt(0x202);
    set_spot_asset_direct(&mut state, contract, market, addr(0x55), short_str("AAA"), felt(0x77));

    let mut ctx = crate::ExecutionContext::new();
    let kind = assets_manager::get_asset_kind(&mut ctx, &state, contract, market).expect("kind");
    assert_eq!(kind, assets_manager::asset_kind_spot());
}

#[test]
fn test_get_asset_kind_unsupported() {
    let state = MockStateReader::new();
    let contract = addr(0x103);
    let market = felt(0x203);

    let mut ctx = crate::ExecutionContext::new();
    let kind = assets_manager::get_asset_kind(&mut ctx, &state, contract, market).expect("kind");
    assert_eq!(kind, assets_manager::ASSET_KIND_UNSUPPORTED);
}

#[test]
fn test_get_base_token_asset() {
    let mut state = MockStateReader::new();
    let contract = addr(0x104);
    let market = felt(0x204);
    let token_addr = addr(0x88);
    let token_name = short_str("USDC");
    set_spot_asset_direct(&mut state, contract, market, token_addr, token_name, felt(0x9));

    let mut ctx = crate::ExecutionContext::new();
    let named = assets_manager::get_base_token_asset(&mut ctx, &state, contract, market).expect("base token");
    assert_eq!(named.token_address, token_addr);
    assert_eq!(named.token_name, token_name);
}

#[test]
fn test_get_asset_kind_reads_root_asset_kind_map() {
    let mut state = MockStateReader::new();
    let contract = addr(0x105);
    let market = felt(0x205);
    set_storage(
        &mut state,
        contract,
        storage_key_for_map("asset_kind", market),
        assets_manager::asset_kind_dated_option(),
    );

    let mut ctx = crate::ExecutionContext::new();
    let kind = assets_manager::get_asset_kind(&mut ctx, &state, contract, market).expect("kind");
    assert_eq!(kind, assets_manager::asset_kind_dated_option());
}

#[test]
fn test_get_asset_kind_dated_option_from_direct_option_asset_map() {
    let mut state = MockStateReader::new();
    let contract = addr(0x106);
    let market = felt(0x206);
    let base = storage_key_for_map("option_asset", market);
    set_storage(&mut state, contract, base, market);
    set_storage(&mut state, contract, storage_key_with_offset(base, 1), short_str("BTC"));
    set_storage(&mut state, contract, storage_key_with_offset(base, 2), short_str("USD"));
    set_storage(&mut state, contract, storage_key_with_offset(base, 3), felt(1));
    set_storage(&mut state, contract, storage_key_with_offset(base, 4), felt(1));
    set_storage(&mut state, contract, storage_key_with_offset(base, 5), felt(63_000));
    set_storage(&mut state, contract, storage_key_with_offset(base, 6), felt(1_786_080_000));

    let mut ctx = crate::ExecutionContext::new();
    let kind = assets_manager::get_asset_kind(&mut ctx, &state, contract, market).expect("kind");
    assert_eq!(kind, assets_manager::asset_kind_dated_option());
}

#[test]
fn test_get_asset_min_size_increment_reads_root_map() {
    let mut state = MockStateReader::new();
    let contract = addr(0x107);
    let market = felt(0x207);
    set_storage(&mut state, contract, storage_key_for_map("asset_min_size_increment", market), felt(100));

    let mut ctx = crate::ExecutionContext::new();
    let increment =
        assets_manager::get_asset_min_size_increment(&mut ctx, &state, contract, market).expect("increment");
    assert_eq!(increment, felt(100));
}
