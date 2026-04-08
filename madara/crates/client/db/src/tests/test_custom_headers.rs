#![cfg(test)]

use crate::MadaraBackend;
use mp_block::header::{CustomHeader, GasPrices};
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
fn get_custom_header_returns_clone_without_consuming_entry() {
    let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));
    let custom_header = sample_custom_header(7, 1);

    backend.set_custom_header(custom_header.clone());

    let first = backend.get_custom_header(custom_header.block_n).expect("custom header should be present");
    let second = backend.get_custom_header(custom_header.block_n).expect("custom header should still be present");
    let taken = backend.take_custom_header(custom_header.block_n).expect("take should still return the entry");

    assert_custom_header_eq(&first, &custom_header);
    assert_custom_header_eq(&second, &custom_header);
    assert_custom_header_eq(&taken, &custom_header);
}

#[test]
fn take_custom_header_removes_entry() {
    let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));
    let custom_header = sample_custom_header(8, 2);

    backend.set_custom_header(custom_header.clone());

    let taken = backend.take_custom_header(custom_header.block_n).expect("take should return the stored header");
    assert_custom_header_eq(&taken, &custom_header);
    assert!(backend.get_custom_header(custom_header.block_n).is_none(), "take should consume the header");
    assert!(backend.take_custom_header(custom_header.block_n).is_none(), "entry should only be consumed once");
}

#[test]
fn set_custom_header_overwrites_existing_entry_for_same_block() {
    let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));
    let initial = sample_custom_header(9, 3);
    let replacement = sample_custom_header(9, 4);

    backend.set_custom_header(initial);
    backend.set_custom_header(replacement.clone());

    let stored = backend.get_custom_header(replacement.block_n).expect("latest header should be stored");
    let taken = backend.take_custom_header(replacement.block_n).expect("take should return latest header");

    assert_custom_header_eq(&stored, &replacement);
    assert_custom_header_eq(&taken, &replacement);
    assert!(backend.get_custom_header(replacement.block_n).is_none(), "take should clear the overwritten entry");
}
