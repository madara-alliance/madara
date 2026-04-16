#![cfg(test)]

use crate::MadaraBackend;
use crate::preconfirmed::PreconfirmedBlock;
use mp_block::header::{CustomHeader, GasPrices};
use mp_block::PreconfirmedHeader;
use mp_chain_config::ChainConfig;
use mp_convert::Felt;
use std::sync::Arc;

fn sample_custom_header(block_n: u64, marker: u64) -> CustomHeader {
    CustomHeader {
        block_n,
        timestamp: 1_700_000_000 + marker,
        gas_prices: GasPrices {
            eth_l1_gas_price: u128::from(10 + marker),
            strk_l1_gas_price: u128::from(20 + marker),
            eth_l1_data_gas_price: u128::from(30 + marker),
            strk_l1_data_gas_price: u128::from(40 + marker),
            eth_l2_gas_price: u128::from(50 + marker),
            strk_l2_gas_price: u128::from(60 + marker),
        },
        expected_block_hash: Felt::from(1_000 + marker),
    }
}

fn assert_custom_header_eq(actual: &CustomHeader, expected: &CustomHeader) {
    assert_eq!(actual.block_n, expected.block_n);
    assert_eq!(actual.timestamp, expected.timestamp);
    assert_eq!(actual.gas_prices, expected.gas_prices);
    assert_eq!(actual.expected_block_hash, expected.expected_block_hash);
}

#[test]
fn custom_header_lifecycle_matches_storage_contract() {
    let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));
    let initial = sample_custom_header(7, 1);
    let replacement = sample_custom_header(7, 2);

    // `get` is a non-destructive peek: callers can inspect the staged header multiple times
    // while the block is still open.
    backend.set_custom_header(initial.clone()).unwrap();

    let first = backend.get_custom_header(initial.block_n).expect("custom header should be present");
    let second = backend.get_custom_header(initial.block_n).expect("custom header should still be present");

    assert_custom_header_eq(&first, &initial);
    assert_custom_header_eq(&second, &initial);

    // `set` for the same block number should overwrite the staged value. This is how replay can
    // replace the header before block close without accumulating stale entries.
    backend.set_custom_header(replacement.clone()).unwrap();

    let stored = backend.get_custom_header(replacement.block_n).expect("latest header should be stored");
    assert_custom_header_eq(&stored, &replacement);

    // `take` is the consuming read used at block close. It should return the latest header once
    // and remove it from storage.
    let taken = backend.take_custom_header(replacement.block_n).expect("take should return latest header");
    assert_custom_header_eq(&taken, &replacement);
    assert!(backend.get_custom_header(replacement.block_n).is_none(), "take should clear the overwritten entry");
    assert!(backend.take_custom_header(replacement.block_n).is_none(), "entry should only be consumed once");
}

#[test]
fn custom_header_updates_live_preconfirmed_header_when_block_is_already_open() {
    let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));
    let original_header = PreconfirmedHeader {
        block_number: 0,
        block_timestamp: 111_u64.into(),
        gas_prices: GasPrices {
            eth_l1_gas_price: 1,
            strk_l1_gas_price: 2,
            eth_l1_data_gas_price: 3,
            strk_l1_data_gas_price: 4,
            eth_l2_gas_price: 5,
            strk_l2_gas_price: 6,
        },
        ..Default::default()
    };
    backend.write_access().new_preconfirmed(PreconfirmedBlock::new(original_header)).unwrap();

    let replacement = sample_custom_header(0, 9);
    backend.set_custom_header(replacement.clone()).unwrap();

    let preconfirmed = backend.block_view_on_preconfirmed().expect("live preconfirmed block should exist");
    assert_eq!(preconfirmed.block().header.block_number, replacement.block_n);
    assert_eq!(preconfirmed.block().header.block_timestamp.0, replacement.timestamp);
    assert_eq!(preconfirmed.block().header.gas_prices, replacement.gas_prices);

    let stored = backend.get_custom_header(replacement.block_n).expect("custom header should remain staged");
    assert_custom_header_eq(&stored, &replacement);
}
