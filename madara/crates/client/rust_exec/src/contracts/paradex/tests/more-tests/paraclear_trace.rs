use crate::contracts::paradex::paraclear;
use crate::state::mock::MockStateReader;
use crate::storage::{storage_key_for_map, storage_key_for_map2, storage_key_with_offset};

use super::super::fixtures::{addr, felt, set_storage};

#[test]
fn test_extract_account_perp_markets_for_trace_tail_order() {
    let mut state = MockStateReader::new();
    let contract = addr(0x9000);
    let account = addr(0x9001);
    let market1 = felt(0x111);
    let market2 = felt(0x222);

    let tail_key = storage_key_for_map("Paraclear_perpetual_asset_balance_tail", account.0);
    set_storage(&mut state, contract, tail_key, market2);

    let base1 = storage_key_for_map2("Paraclear_perpetual_asset_balance", account.0, market1);
    let base2 = storage_key_for_map2("Paraclear_perpetual_asset_balance", account.0, market2);

    set_storage(&mut state, contract, base1, market1);
    set_storage(&mut state, contract, base2, market2);
    set_storage(&mut state, contract, storage_key_with_offset(base2, 4), market1);
    set_storage(&mut state, contract, storage_key_with_offset(base1, 4), felt(0));

    let markets =
        paraclear::extract_account_perp_markets_for_trace(&state, contract, account).expect("extract markets");
    assert_eq!(markets, vec![market2, market1]);
}
