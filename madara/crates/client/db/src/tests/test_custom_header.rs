#![cfg(any(test, feature = "testing"))]

use crate::{test_utils::add_test_block, MadaraBackend};
use mp_block::header::{CustomHeader, GasPrices};
use mp_chain_config::ChainConfig;
use mp_convert::Felt;
use std::sync::Arc;

fn custom_header(block_n: u64, timestamp: u64, gas_seed: u128) -> CustomHeader {
    CustomHeader {
        block_n,
        timestamp,
        gas_prices: GasPrices {
            eth_l1_gas_price: gas_seed,
            strk_l1_gas_price: gas_seed + 1,
            eth_l1_data_gas_price: gas_seed + 2,
            strk_l1_data_gas_price: gas_seed + 3,
            eth_l2_gas_price: gas_seed + 4,
            strk_l2_gas_price: gas_seed + 5,
        },
        expected_block_hash: Felt::from(block_n + 1000),
    }
}

fn assert_custom_header(actual: Option<CustomHeader>, expected: Option<&CustomHeader>) {
    match (actual, expected) {
        (Some(actual), Some(expected)) => {
            assert_eq!(actual.block_n, expected.block_n);
            assert_eq!(actual.timestamp, expected.timestamp);
            assert_eq!(actual.gas_prices, expected.gas_prices);
            assert_eq!(actual.expected_block_hash, expected.expected_block_hash);
        }
        (None, None) => {}
        (actual, expected) => panic!(
            "custom header mismatch: actual_present={}, expected_present={}",
            actual.is_some(),
            expected.is_some()
        ),
    }
}

#[test]
fn custom_headers_are_stored_and_cleared_per_block() {
    let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));

    let header_1 = custom_header(1, 111, 10);
    let header_2 = custom_header(2, 222, 20);

    backend.set_custom_header(header_1.clone());
    backend.set_custom_header(header_2.clone());

    assert_custom_header(backend.get_custom_header(1), Some(&header_1));
    assert_custom_header(backend.get_custom_header(2), Some(&header_2));

    assert_custom_header(backend.get_custom_header_with_clear(1, true), Some(&header_1));
    assert_custom_header(backend.get_custom_header(1), None);
    assert_custom_header(backend.get_custom_header(2), Some(&header_2));
}

#[test]
fn finalizing_one_block_only_clears_that_blocks_custom_header() {
    let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));

    add_test_block(&backend, 0, vec![]);

    let header_1 = custom_header(1, 1_111, 100);
    let header_2 = custom_header(2, 2_222, 200);
    backend.set_custom_header(header_1.clone());
    backend.set_custom_header(header_2.clone());

    let preconfirmed_1 = backend.block_view_on_preconfirmed_or_fake().expect("fake preconfirmed #1");
    let info_1 = preconfirmed_1.get_block_info();
    assert_eq!(info_1.header.block_number, 1);
    assert_eq!(info_1.header.block_timestamp.0, header_1.timestamp);
    assert_eq!(info_1.header.gas_prices, header_1.gas_prices);

    add_test_block(&backend, 1, vec![]);

    assert_custom_header(backend.get_custom_header(1), None);
    assert_custom_header(backend.get_custom_header(2), Some(&header_2));

    let preconfirmed_2 = backend.block_view_on_preconfirmed_or_fake().expect("fake preconfirmed #2");
    let info_2 = preconfirmed_2.get_block_info();
    assert_eq!(info_2.header.block_number, 2);
    assert_eq!(info_2.header.block_timestamp.0, header_2.timestamp);
    assert_eq!(info_2.header.gas_prices, header_2.gas_prices);
}
