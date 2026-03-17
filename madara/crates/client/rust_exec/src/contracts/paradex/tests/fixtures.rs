#![allow(dead_code)]

use starknet_types_core::felt::Felt;

use crate::contracts::paradex_codegen::account_component_layout as account_layout;
use crate::contracts::paradex_codegen::paraclear_layout;
use crate::contracts::paradex_codegen::paraclear_types::{FeeWithCapRequest, OrderCategory, OrderV3, TradeRequestV3};
use crate::contracts::paradex_codegen::token_component_layout as token_layout;
use crate::state::mock::MockStateReader;
use crate::storage::{
    short_string_to_felt, storage_key_for_map, storage_key_for_map2, storage_key_for_map_poseidon,
    storage_key_for_substorage_map_poseidon, storage_key_for_substorage_map_poseidon_add,
    storage_key_for_substorage_map_poseidon_hash, storage_key_with_offset,
};
use crate::types::{ContractAddress, StorageKey};

pub fn felt(value: u64) -> Felt {
    Felt::from(value)
}

pub fn addr(value: u64) -> ContractAddress {
    ContractAddress(Felt::from(value))
}

pub const SCALE: i128 = 100_000_000;

pub fn i128_to_felt(value: i128) -> Felt {
    if value >= 0 {
        Felt::from(value as u128)
    } else {
        Felt::ZERO - Felt::from((-value) as u128)
    }
}

pub fn set_storage(state: &mut MockStateReader, contract: ContractAddress, key: StorageKey, value: Felt) {
    state.set_storage(contract, key, value);
}

pub fn set_spot_asset_direct(
    state: &mut MockStateReader,
    contract: ContractAddress,
    market: Felt,
    base_token_address: ContractAddress,
    base_token_name: Felt,
    quote_asset: Felt,
) {
    let base = storage_key_for_map_poseidon("spot_asset", market);
    set_storage(state, contract, base, market);
    set_storage(state, contract, storage_key_with_offset(base, 1), base_token_address.0);
    set_storage(state, contract, storage_key_with_offset(base, 2), base_token_name);
    set_storage(state, contract, storage_key_with_offset(base, 3), quote_asset);
}

pub fn set_spot_asset_substorage(
    state: &mut MockStateReader,
    contract: ContractAddress,
    base_key: StorageKey,
    market: Felt,
    base_token_address: ContractAddress,
    base_token_name: Felt,
    quote_asset: Felt,
) {
    let base = storage_key_for_substorage_map_poseidon(base_key, "spot_asset", market);
    set_storage(state, contract, base, market);
    set_storage(state, contract, storage_key_with_offset(base, 1), base_token_address.0);
    set_storage(state, contract, storage_key_with_offset(base, 2), base_token_name);
    set_storage(state, contract, storage_key_with_offset(base, 3), quote_asset);
}

pub fn set_future_asset_direct(
    state: &mut MockStateReader,
    contract: ContractAddress,
    market: Felt,
    base_asset: Felt,
    quote_asset: Felt,
    tick_size: Felt,
    imf_base: Felt,
    imf_factor: Felt,
    mmf_factor: Felt,
    imf_shift: Felt,
) {
    let base = storage_key_for_map_poseidon("perpetual_future_asset", market);
    set_storage(state, contract, base, market);
    set_storage(state, contract, storage_key_with_offset(base, 1), base_asset);
    set_storage(state, contract, storage_key_with_offset(base, 2), quote_asset);
    set_storage(state, contract, storage_key_with_offset(base, 3), tick_size);
    set_storage(state, contract, storage_key_with_offset(base, 4), imf_base);
    set_storage(state, contract, storage_key_with_offset(base, 5), imf_factor);
    set_storage(state, contract, storage_key_with_offset(base, 6), mmf_factor);
    set_storage(state, contract, storage_key_with_offset(base, 7), imf_shift);
}

pub fn set_option_asset_direct(
    state: &mut MockStateReader,
    contract: ContractAddress,
    market: Felt,
    base_asset: Felt,
    quote_asset: Felt,
    tick_size: Felt,
    option_type: Felt,
    strike: Felt,
) {
    let base = storage_key_for_map_poseidon("perpetual_option_asset", market);
    set_storage(state, contract, base, market);
    set_storage(state, contract, storage_key_with_offset(base, 1), base_asset);
    set_storage(state, contract, storage_key_with_offset(base, 2), quote_asset);
    set_storage(state, contract, storage_key_with_offset(base, 3), tick_size);
    set_storage(state, contract, storage_key_with_offset(base, 4), option_type);
    set_storage(state, contract, storage_key_with_offset(base, 5), strike);
}

pub fn set_future_asset_substorage_add(
    state: &mut MockStateReader,
    contract: ContractAddress,
    base_key: StorageKey,
    market: Felt,
    base_asset: Felt,
) {
    let base = storage_key_for_substorage_map_poseidon_add(base_key, "perpetual_future_asset", market);
    set_storage(state, contract, base, market);
    set_storage(state, contract, storage_key_with_offset(base, 1), base_asset);
}

pub fn set_option_asset_substorage_hash(
    state: &mut MockStateReader,
    contract: ContractAddress,
    base_key: StorageKey,
    market: Felt,
    base_asset: Felt,
) {
    let base = storage_key_for_substorage_map_poseidon_hash(base_key, "perpetual_option_asset", market);
    set_storage(state, contract, base, market);
    set_storage(state, contract, storage_key_with_offset(base, 1), base_asset);
}

pub fn set_oracle_latest_tick_data(
    state: &mut MockStateReader,
    contract: ContractAddress,
    market: Felt,
    asset_key: Felt,
    asset_value: Felt,
    decimals: Felt,
) {
    let base = storage_key_for_map("latest_tick_data", market);
    set_storage(state, contract, base, asset_key);
    set_storage(state, contract, storage_key_with_offset(base, 1), asset_value);
    set_storage(state, contract, storage_key_with_offset(base, 2), decimals);
}

pub fn set_oracle_funding_index(
    state: &mut MockStateReader,
    contract: ContractAddress,
    market: Felt,
    funding_index: Felt,
) {
    let base = storage_key_for_map("funding_index_data", market);
    set_storage(state, contract, base, market);
    set_storage(state, contract, storage_key_with_offset(base, 1), funding_index);
}

pub fn short_str(value: &str) -> Felt {
    short_string_to_felt(value)
}

pub fn set_paraclear_dependencies(
    state: &mut MockStateReader,
    contract: ContractAddress,
    assets_manager: ContractAddress,
    oracle: ContractAddress,
) {
    set_storage(state, contract, *paraclear_layout::ASSETS_MANAGER_BASE, assets_manager.0);
    set_storage(state, contract, *paraclear_layout::PARACLEAR_ORACLE_CONTRACT_ADDRESS_BASE, oracle.0);
}

pub fn set_settlement_token(
    state: &mut MockStateReader,
    contract: ContractAddress,
    token_address: ContractAddress,
    token_name: Felt,
) {
    let base = *paraclear_layout::PARACLEAR_SETTLEMENT_TOKEN_ASSET_BASE;
    set_storage(state, contract, storage_key_with_offset(base, 4), token_address.0);
    set_storage(state, contract, storage_key_with_offset(base, 5), token_name);
}

pub fn set_token_name(
    state: &mut MockStateReader,
    contract: ContractAddress,
    token_address: ContractAddress,
    token_name: Felt,
) {
    let base = token_layout::Paraclear_token_asset_key(token_address.0);
    set_storage(state, contract, storage_key_with_offset(base, 5), token_name);
}

pub fn set_token_balance(
    state: &mut MockStateReader,
    contract: ContractAddress,
    account: ContractAddress,
    token_address: ContractAddress,
    amount: i128,
) {
    let tail_key = storage_key_for_map("Paraclear_token_asset_balance_tail", account.0);
    set_storage(state, contract, tail_key, token_address.0);
    let base = storage_key_for_map2("Paraclear_token_asset_balance", account.0, token_address.0);
    set_storage(state, contract, base, token_address.0);
    set_storage(state, contract, storage_key_with_offset(base, 1), i128_to_felt(amount));
    set_storage(state, contract, storage_key_with_offset(base, 2), Felt::ZERO);
    set_storage(state, contract, storage_key_with_offset(base, 3), Felt::ZERO);
}

pub fn set_token_balance_amount_only(
    state: &mut MockStateReader,
    contract: ContractAddress,
    account: ContractAddress,
    token_address: ContractAddress,
    amount: i128,
) {
    let base = storage_key_for_map2("Paraclear_token_asset_balance", account.0, token_address.0);
    set_storage(state, contract, storage_key_with_offset(base, 1), i128_to_felt(amount));
}

pub fn set_account_fee_rate_spot(
    state: &mut MockStateReader,
    contract: ContractAddress,
    account: ContractAddress,
    maker_rate: i128,
    taker_rate: i128,
) {
    let base = account_layout::Paraclear_account_fee_rate_spot_key(account.0);
    set_storage(state, contract, base, Felt::ONE);
    set_storage(state, contract, storage_key_with_offset(base, 1), i128_to_felt(maker_rate));
    set_storage(state, contract, storage_key_with_offset(base, 2), i128_to_felt(taker_rate));
}

pub fn set_account_fee_rate_future(
    state: &mut MockStateReader,
    contract: ContractAddress,
    account: ContractAddress,
    maker_rate: i128,
    taker_rate: i128,
) {
    let base = account_layout::Paraclear_account_fee_rate_key(account.0);
    set_storage(state, contract, base, Felt::ONE);
    set_storage(state, contract, storage_key_with_offset(base, 1), i128_to_felt(maker_rate));
    set_storage(state, contract, storage_key_with_offset(base, 2), i128_to_felt(taker_rate));
}

pub fn set_account_referral(
    state: &mut MockStateReader,
    contract: ContractAddress,
    account: ContractAddress,
    referrer: ContractAddress,
    fee_commission: i128,
    fee_discount: i128,
) {
    let base = account_layout::Paraclear_account_referral_key(account.0);
    set_storage(state, contract, base, referrer.0);
    set_storage(state, contract, storage_key_with_offset(base, 1), i128_to_felt(fee_commission));
    set_storage(state, contract, storage_key_with_offset(base, 2), i128_to_felt(fee_discount));
}

pub fn set_fee_share(
    state: &mut MockStateReader,
    contract: ContractAddress,
    fee_share_account: ContractAddress,
    fee_share_percentage: i128,
) {
    set_storage(state, contract, *paraclear_layout::PARACLEAR_FEE_SHARE_ACCOUNT_ADDRESS_BASE, fee_share_account.0);
    set_storage(
        state,
        contract,
        *paraclear_layout::PARACLEAR_FEE_SHARE_PERCENTAGE_BASE,
        i128_to_felt(fee_share_percentage),
    );
}

pub fn sample_order(category: OrderCategory) -> OrderV3 {
    OrderV3 {
        account: addr(0x1111),
        market: felt(0x2222),
        side: felt(1),
        orderType: felt(0),
        size: felt(100),
        price: felt(200),
        signature_timestamp: felt(1234),
        is_reduce_only: false,
        order_category: category,
    }
}

pub fn sample_trade(category_maker: OrderCategory, category_taker: OrderCategory) -> TradeRequestV3 {
    TradeRequestV3 {
        id: felt(0x10),
        size: felt(5),
        price: felt(100),
        traded_at: felt(1000),
        maker_order: sample_order(category_maker),
        taker_order: OrderV3 { account: addr(0x2222), ..sample_order(category_taker) },
    }
}

pub fn encode_order_category_for_test(cat: &OrderCategory) -> Vec<Felt> {
    match cat {
        OrderCategory::Unspecified => vec![felt(0)],
        OrderCategory::API => vec![felt(1)],
        OrderCategory::RPI => vec![felt(2)],
        OrderCategory::Interactive => vec![felt(3)],
        OrderCategory::Dynamic(fee) => vec![felt(4), fee.fee, fee.fee_cap, fee.fee_floor],
    }
}

pub fn encode_order_v3_for_test(order: &OrderV3) -> Vec<Felt> {
    let mut data = Vec::new();
    data.push(order.account.0);
    data.push(order.market);
    data.push(order.side);
    data.push(order.orderType);
    data.push(order.size);
    data.push(order.price);
    data.push(order.signature_timestamp);
    data.push(if order.is_reduce_only { Felt::ONE } else { Felt::ZERO });
    data.extend(encode_order_category_for_test(&order.order_category));
    data
}

pub fn encode_trade_request_v3_for_test(trade: &TradeRequestV3) -> Vec<Felt> {
    let mut data = Vec::new();
    data.push(trade.id);
    data.push(trade.size);
    data.push(trade.price);
    data.push(trade.traded_at);
    data.extend(encode_order_v3_for_test(&trade.maker_order));
    data.extend(encode_order_v3_for_test(&trade.taker_order));
    data
}

pub fn dynamic_fee_request() -> FeeWithCapRequest {
    FeeWithCapRequest { fee: felt(7), fee_cap: felt(8), fee_floor: felt(9) }
}
