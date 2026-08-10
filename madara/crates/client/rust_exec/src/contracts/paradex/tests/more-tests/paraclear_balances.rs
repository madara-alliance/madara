#![allow(clippy::identity_op, clippy::neg_multiply)]

use super::super::fixtures::{addr, felt, i128_to_felt, set_storage, SCALE};
use crate::contracts::paradex::paraclear;
use crate::core::state::mock::MockStateReader;
use crate::core::storage::{storage_key_for_map, storage_key_for_map2, storage_key_with_offset};

#[test]
fn test_read_token_balance_amount_missing_zero() {
    let state = MockStateReader::new();
    let contract = addr(0x400);
    let account = addr(0x401);
    let token = addr(0x402);

    let mut ctx = crate::ExecutionContext::new();
    let amount =
        paraclear::read_token_balance_amount_for_test(&mut ctx, &state, contract, account, token).expect("read amount");
    assert_eq!(amount, 0);
}

#[test]
fn test_upsert_token_balance_inserts_new() {
    let mut state = MockStateReader::new();
    let contract = addr(0x410);
    let account = addr(0x411);
    let token = addr(0x412);

    let tail_key = storage_key_for_map("Paraclear_token_asset_balance_tail", account.0);
    set_storage(&mut state, contract, tail_key, felt(0));
    let base = storage_key_for_map2("Paraclear_token_asset_balance", account.0, token.0);
    set_storage(&mut state, contract, storage_key_with_offset(base, 2), felt(0x1));
    set_storage(&mut state, contract, storage_key_with_offset(base, 3), felt(0x2));

    let mut ctx = crate::ExecutionContext::new();
    ctx.storage_read(&state, contract, storage_key_with_offset(base, 2)).expect("read prev");
    ctx.storage_read(&state, contract, storage_key_with_offset(base, 3)).expect("read next");
    paraclear::upsert_token_balance_for_test(&mut ctx, &state, contract, account, token, 5 * 100_000_000)
        .expect("upsert");

    let result = ctx.build_result();
    let updates = result.state_diff.storage_updates.get(&contract).expect("updates");
    let base = storage_key_for_map2("Paraclear_token_asset_balance", account.0, token.0);

    let mut map = std::collections::HashMap::new();
    for (key, value) in updates {
        map.insert(*key, *value);
    }

    assert_eq!(map.get(&base).copied(), Some(token.0));
    assert_eq!(map.get(&storage_key_with_offset(base, 1)).copied(), Some(felt(5 * 100_000_000u64)));
    assert_eq!(map.get(&storage_key_with_offset(base, 2)).copied(), Some(felt(0)));
    assert_eq!(map.get(&storage_key_with_offset(base, 3)).copied(), Some(felt(0)));
    assert_eq!(map.get(&tail_key).copied(), Some(token.0));
}

#[test]
fn test_upsert_token_balance_updates_existing() {
    let mut state = MockStateReader::new();
    let contract = addr(0x420);
    let account = addr(0x421);
    let token = addr(0x422);

    let base = storage_key_for_map2("Paraclear_token_asset_balance", account.0, token.0);
    set_storage(&mut state, contract, base, token.0);
    set_storage(&mut state, contract, storage_key_with_offset(base, 1), felt(3 * 100_000_000u64));

    let mut ctx = crate::ExecutionContext::new();
    paraclear::upsert_token_balance_for_test(&mut ctx, &state, contract, account, token, 2 * 100_000_000)
        .expect("upsert");

    let result = ctx.build_result();
    let updates = result.state_diff.storage_updates.get(&contract).expect("updates");

    assert_eq!(updates.len(), 1);
    assert_eq!(updates.get(&storage_key_with_offset(base, 1)).copied(), Some(felt(5 * 100_000_000u64)));
}

#[test]
fn test_update_token_balance_writes_delta() {
    let mut state = MockStateReader::new();
    let contract = addr(0x430);
    let account = addr(0x431);
    let token = addr(0x432);

    let base = storage_key_for_map2("Paraclear_token_asset_balance", account.0, token.0);
    set_storage(&mut state, contract, base, token.0);
    set_storage(&mut state, contract, storage_key_with_offset(base, 1), felt(1 * 100_000_000u64));

    let mut ctx = crate::ExecutionContext::new();
    paraclear::update_token_balance_for_test(&mut ctx, &state, contract, account, token, -1 * 100_000_000)
        .expect("update");

    let result = ctx.build_result();
    let updates = result.state_diff.storage_updates.get(&contract).expect("updates");

    assert_eq!(updates.len(), 1);
    assert_eq!(updates.get(&storage_key_with_offset(base, 1)).copied(), Some(felt(0)));
}

#[test]
fn test_resolve_token_balance_base() {
    let state = MockStateReader::new();
    let contract = addr(0x440);
    let account = addr(0x441);
    let token = addr(0x442);

    let mut ctx = crate::ExecutionContext::new();
    let first =
        paraclear::resolve_token_balance_base_for_test(&state, &mut ctx, contract, account, token).expect("base");
    let second =
        paraclear::resolve_token_balance_base_for_test(&state, &mut ctx, contract, account, token).expect("base");
    assert_eq!(first, second);
}

#[test]
fn test_read_max_pending_transfer_executions_default() {
    let state = MockStateReader::new();
    let contract = addr(0x450);

    let mut ctx = crate::ExecutionContext::new();
    let value = paraclear::read_max_pending_transfer_executions_for_test(&state, &mut ctx, contract).expect("value");
    assert_eq!(value, 0);
}

#[test]
fn test_add_pending_transfer_writes() {
    let mut state = MockStateReader::new();
    let contract = addr(0x460);
    let executor = addr(0x461);
    let trade_id = felt(0x1234);
    let recipient = addr(0x462);
    let token = addr(0x463);
    let amount = 5 * 100_000_000i128;

    // Ensure count starts at zero.
    let count_key =
        crate::contracts::paradex::schema::paraclear_layout::pending_transfer_count_by_executor_key(executor.0);
    set_storage(&mut state, contract, count_key, felt(0));

    let mut ctx = crate::ExecutionContext::new();
    paraclear::add_pending_transfer_for_test(&state, &mut ctx, contract, executor, trade_id, recipient, token, amount)
        .expect("add transfer");

    let result = ctx.build_result();
    let updates = result.state_diff.storage_updates.get(&contract).expect("updates");

    let mut map = std::collections::HashMap::new();
    for (key, value) in updates {
        map.insert(*key, *value);
    }

    let base = crate::contracts::paradex::schema::paraclear_layout::pending_transfers_key2(executor.0, felt(1));
    assert_eq!(map.get(&count_key).copied(), Some(felt(1)));
    assert_eq!(map.get(&base).copied(), Some(trade_id));
    assert_eq!(map.get(&storage_key_with_offset(base, 1)).copied(), Some(recipient.0));
    assert_eq!(map.get(&storage_key_with_offset(base, 2)).copied(), Some(token.0));
    assert_eq!(map.get(&storage_key_with_offset(base, 3)).copied(), Some(felt(5 * 100_000_000u64)));
}

#[test]
fn test_read_perpetual_balance_missing_zero() {
    let state = MockStateReader::new();
    let contract = addr(0x470);
    let account = addr(0x471);
    let market = felt(0x472);

    let mut ctx = crate::ExecutionContext::new();
    let (market_v, amount, cost, cached_funding, prev, next) =
        paraclear::read_perpetual_balance_fields_for_test(&mut ctx, &state, contract, account, market, 0)
            .expect("read balance");

    assert_eq!(market_v, felt(0));
    assert_eq!(amount, felt(0));
    assert_eq!(cost, felt(0));
    assert_eq!(cached_funding, felt(0));
    assert_eq!(prev, felt(0));
    assert_eq!(next, felt(0));
}

#[test]
fn test_create_perpetual_balance_inserts_new() {
    let mut state = MockStateReader::new();
    let contract = addr(0x480);
    let account = addr(0x481);
    let market = felt(0x482);

    let base = storage_key_for_map2("Paraclear_perpetual_asset_balance", account.0, market);
    set_storage(&mut state, contract, storage_key_with_offset(base, 4), felt(0x1));
    set_storage(&mut state, contract, storage_key_with_offset(base, 5), felt(0x2));

    let tail_key = storage_key_for_map("Paraclear_perpetual_asset_balance_tail", account.0);
    set_storage(&mut state, contract, tail_key, felt(0));

    let mut ctx = crate::ExecutionContext::new();
    ctx.storage_read(&state, contract, storage_key_with_offset(base, 4)).expect("read prev");
    ctx.storage_read(&state, contract, storage_key_with_offset(base, 5)).expect("read next");
    paraclear::upsert_perpetual_balance_for_test(
        &mut ctx,
        &state,
        contract,
        account,
        market,
        5 * SCALE,
        2 * SCALE,
        1 * SCALE,
        0,
    )
    .expect("upsert");

    let result = ctx.build_result();
    let updates = result.state_diff.storage_updates.get(&contract).expect("updates");

    let mut base_ctx = crate::ExecutionContext::new();
    let base =
        paraclear::resolve_perp_balance_base_for_test(&state, &mut base_ctx, contract, account, market).expect("base");

    assert_eq!(updates.get(&base).copied(), Some(market));
    assert_eq!(updates.get(&storage_key_with_offset(base, 1)).copied(), Some(i128_to_felt(5 * SCALE)));
    assert_eq!(updates.get(&storage_key_with_offset(base, 2)).copied(), Some(i128_to_felt(2 * SCALE)));
    assert_eq!(updates.get(&storage_key_with_offset(base, 3)).copied(), Some(i128_to_felt(1 * SCALE)));
    assert_eq!(updates.get(&storage_key_with_offset(base, 4)).copied(), Some(felt(0)));
    assert_eq!(updates.get(&storage_key_with_offset(base, 5)).copied(), Some(felt(0)));
    assert_eq!(updates.get(&tail_key).copied(), Some(market));
}

#[test]
fn test_ensure_perpetual_balance_creates_empty_position() {
    let mut state = MockStateReader::new();
    let contract = addr(0x485);
    let account = addr(0x486);
    let market = felt(0x487);

    let tail_key = storage_key_for_map("Paraclear_perpetual_asset_balance_tail", account.0);
    set_storage(&mut state, contract, tail_key, felt(0));

    let mut ctx = crate::ExecutionContext::new();
    paraclear::ensure_perpetual_balance_for_test(&mut ctx, &state, contract, account, market, 0).expect("ensure");

    let result = ctx.build_result();
    let updates = result.state_diff.storage_updates.get(&contract).expect("updates");

    let mut base_ctx = crate::ExecutionContext::new();
    let base =
        paraclear::resolve_perp_balance_base_for_test(&state, &mut base_ctx, contract, account, market).expect("base");

    assert_eq!(updates.get(&base).copied(), Some(market));
    assert_eq!(updates.get(&tail_key).copied(), Some(market));
    assert_eq!(updates.get(&storage_key_with_offset(base, 1)).copied(), None);
    assert_eq!(updates.get(&storage_key_with_offset(base, 2)).copied(), None);
    assert_eq!(updates.get(&storage_key_with_offset(base, 3)).copied(), None);
    assert_eq!(updates.get(&storage_key_with_offset(base, 4)).copied(), None);
    assert_eq!(updates.get(&storage_key_with_offset(base, 5)).copied(), None);
}

#[test]
fn test_upsert_perpetual_balance_updates_existing() {
    let mut state = MockStateReader::new();
    let contract = addr(0x490);
    let account = addr(0x491);
    let market = felt(0x492);

    let base = storage_key_for_map2("Paraclear_perpetual_asset_balance", account.0, market);
    set_storage(&mut state, contract, base, felt(0xdead));
    set_storage(&mut state, contract, storage_key_with_offset(base, 1), i128_to_felt(2 * SCALE));
    set_storage(&mut state, contract, storage_key_with_offset(base, 2), i128_to_felt(3 * SCALE));
    set_storage(&mut state, contract, storage_key_with_offset(base, 3), i128_to_felt(4 * SCALE));
    set_storage(&mut state, contract, storage_key_with_offset(base, 4), felt(0x11));
    set_storage(&mut state, contract, storage_key_with_offset(base, 5), felt(0x12));

    let mut ctx = crate::ExecutionContext::new();
    ctx.storage_read(&state, contract, storage_key_with_offset(base, 4)).expect("read prev");
    ctx.storage_read(&state, contract, storage_key_with_offset(base, 5)).expect("read next");
    paraclear::upsert_perpetual_balance_for_test(
        &mut ctx,
        &state,
        contract,
        account,
        market,
        5 * SCALE,
        6 * SCALE,
        7 * SCALE,
        0,
    )
    .expect("upsert");

    let result = ctx.build_result();
    let updates = result.state_diff.storage_updates.get(&contract).expect("updates");

    assert_eq!(updates.len(), 6);
    assert_eq!(updates.get(&base).copied(), Some(market));
    assert_eq!(updates.get(&storage_key_with_offset(base, 1)).copied(), Some(i128_to_felt(5 * SCALE)));
    assert_eq!(updates.get(&storage_key_with_offset(base, 2)).copied(), Some(i128_to_felt(6 * SCALE)));
    assert_eq!(updates.get(&storage_key_with_offset(base, 3)).copied(), Some(i128_to_felt(7 * SCALE)));
}

#[test]
fn test_remove_perpetual_balance_unlinks() {
    let mut state = MockStateReader::new();
    let contract = addr(0x4a0);
    let account = addr(0x4a1);
    let market1 = felt(0x4a2);
    let market2 = felt(0x4a3);

    let tail_key = storage_key_for_map("Paraclear_perpetual_asset_balance_tail", account.0);
    set_storage(&mut state, contract, tail_key, market2);

    let base1 = storage_key_for_map2("Paraclear_perpetual_asset_balance", account.0, market1);
    set_storage(&mut state, contract, base1, market1);
    set_storage(&mut state, contract, storage_key_with_offset(base1, 1), i128_to_felt(1 * SCALE));
    set_storage(&mut state, contract, storage_key_with_offset(base1, 2), i128_to_felt(2 * SCALE));
    set_storage(&mut state, contract, storage_key_with_offset(base1, 3), i128_to_felt(3 * SCALE));
    set_storage(&mut state, contract, storage_key_with_offset(base1, 4), felt(0));
    set_storage(&mut state, contract, storage_key_with_offset(base1, 5), market2);

    let base2 = storage_key_for_map2("Paraclear_perpetual_asset_balance", account.0, market2);
    set_storage(&mut state, contract, base2, market2);
    set_storage(&mut state, contract, storage_key_with_offset(base2, 1), i128_to_felt(4 * SCALE));
    set_storage(&mut state, contract, storage_key_with_offset(base2, 2), i128_to_felt(5 * SCALE));
    set_storage(&mut state, contract, storage_key_with_offset(base2, 3), i128_to_felt(6 * SCALE));
    set_storage(&mut state, contract, storage_key_with_offset(base2, 4), market1);
    set_storage(&mut state, contract, storage_key_with_offset(base2, 5), felt(0x99));

    let mut ctx = crate::ExecutionContext::new();
    ctx.storage_read(&state, contract, storage_key_with_offset(base1, 5)).expect("read base1 next");
    ctx.storage_read(&state, contract, base2).expect("read base2");
    ctx.storage_read(&state, contract, storage_key_with_offset(base2, 1)).expect("read amount");
    ctx.storage_read(&state, contract, storage_key_with_offset(base2, 2)).expect("read cost");
    ctx.storage_read(&state, contract, storage_key_with_offset(base2, 3)).expect("read funding");
    let next_key = storage_key_with_offset(base2, 5);
    ctx.storage_read(&state, contract, next_key).expect("read next");
    ctx.storage_write(contract, next_key, felt(0));
    paraclear::remove_perpetual_balance_for_test(&mut ctx, &state, contract, account, market2, 0).expect("remove");

    let result = ctx.build_result();
    let updates = result.state_diff.storage_updates.get(&contract).expect("updates");

    assert_eq!(updates.get(&storage_key_with_offset(base1, 5)).copied(), Some(felt(0)));
    assert_eq!(updates.get(&tail_key).copied(), Some(market1));
    assert_eq!(updates.get(&base2).copied(), Some(felt(0)));
    assert_eq!(updates.get(&storage_key_with_offset(base2, 1)).copied(), Some(felt(0)));
    assert_eq!(updates.get(&storage_key_with_offset(base2, 2)).copied(), Some(felt(0)));
    assert_eq!(updates.get(&storage_key_with_offset(base2, 3)).copied(), Some(felt(0)));
    assert_eq!(updates.get(&storage_key_with_offset(base2, 4)).copied(), Some(felt(0)));
    assert_eq!(updates.get(&storage_key_with_offset(base2, 5)).copied(), Some(felt(0)));
}

#[test]
fn test_upsert_total_realized_pnl_and_funding_writes() {
    let mut state = MockStateReader::new();
    let contract = addr(0x4b0);
    let account = addr(0x4b1);
    let market = felt(0x4b2);

    let base = paraclear::perp_map2_key_for_test(0, "invariants_perpetual_asset_info", market, account.0);
    set_storage(&mut state, contract, base, i128_to_felt(2 * SCALE));
    set_storage(&mut state, contract, storage_key_with_offset(base, 1), i128_to_felt(3 * SCALE));

    let mut ctx = crate::ExecutionContext::new();
    paraclear::upsert_total_realized_pnl_and_funding_for_test(
        &mut ctx,
        &state,
        contract,
        market,
        account,
        1 * SCALE,
        2 * SCALE,
        0,
    )
    .expect("upsert pnl");

    let result = ctx.build_result();
    let updates = result.state_diff.storage_updates.get(&contract).expect("updates");

    assert_eq!(updates.get(&base).copied(), Some(i128_to_felt(3 * SCALE)));
    assert_eq!(updates.get(&storage_key_with_offset(base, 1)).copied(), Some(i128_to_felt(5 * SCALE)));
}
