#![cfg(test)]
use crate::preconfirmed::PreconfirmedBlock;
use crate::rocksdb::RocksDBConfig;
use crate::{MadaraBackend, MadaraBackendConfig};
use mp_block::header::{CustomHeader, GasPrices, PreconfirmedHeader};
use mp_block::FullBlockWithoutCommitments;
use mp_chain_config::ChainConfig;
use mp_convert::Felt;
use std::sync::Arc;

fn test_block(block_number: u64) -> FullBlockWithoutCommitments {
    FullBlockWithoutCommitments {
        header: PreconfirmedHeader { block_number, ..Default::default() },
        state_diff: Default::default(),
        transactions: vec![],
        events: vec![],
    }
}

fn discover_block_hash(chain_config: Arc<ChainConfig>, block_number: u64) -> Felt {
    let backend = MadaraBackend::open_for_testing(chain_config);
    backend
        .write_access()
        .add_full_block_with_classes(&test_block(block_number), &[], false)
        .expect("Block add should succeed")
        .block_hash
}

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
fn no_custom_header_block_succeeds_normally() {
    let backend = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());

    let result = backend.write_access().add_full_block_with_classes(&test_block(0), &[], false);

    assert!(result.is_ok());
    assert_eq!(backend.latest_confirmed_block_n(), Some(0));
}

#[test]
fn custom_header_hash_match_succeeds() {
    let chain_config: Arc<ChainConfig> = ChainConfig::madara_test().into();
    let known_hash = discover_block_hash(chain_config.clone(), 0);

    let backend = MadaraBackend::open_for_testing(chain_config);
    backend
        .set_custom_header(CustomHeader { block_n: 0, expected_block_hash: known_hash, ..Default::default() })
        .expect("custom header should be staged");

    let result = backend.write_access().add_full_block_with_classes(&test_block(0), &[], false);

    assert!(result.is_ok());
    assert_eq!(result.unwrap().block_hash, known_hash);
    assert_eq!(backend.latest_confirmed_block_n(), Some(0));
}

#[test]
fn custom_header_hash_mismatch_returns_error() {
    let backend = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());
    backend
        .set_custom_header(CustomHeader {
            block_n: 0,
            expected_block_hash: Felt::from_hex_unchecked("0xdeadbeef"),
            ..Default::default()
        })
        .expect("custom header should be staged");

    let result = backend.write_access().add_full_block_with_classes(&test_block(0), &[], false);

    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("Block hash mismatch"), "Expected hash mismatch error, got: {err_msg}");
}

#[test]
fn custom_header_for_future_block_is_ignored_while_closing_current_block() {
    let backend = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());
    backend
        .set_custom_header(CustomHeader {
            block_n: 5,
            expected_block_hash: Felt::from_hex_unchecked("0x1234"),
            ..Default::default()
        })
        .expect("custom header should be staged");

    let result = backend.write_access().add_full_block_with_classes(&test_block(0), &[], false);

    assert!(result.is_ok());
    assert_eq!(backend.latest_confirmed_block_n(), Some(0));
    assert!(backend.get_custom_header(5).is_some(), "Future custom header should remain staged");
}

#[test]
fn custom_header_for_future_block_is_not_consumed() {
    let backend = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());
    backend
        .set_custom_header(CustomHeader {
            block_n: 5,
            expected_block_hash: Felt::from_hex_unchecked("0x1234"),
            ..Default::default()
        })
        .expect("custom header should be staged");

    let _ = backend.write_access().add_full_block_with_classes(&test_block(0), &[], false);

    let header = backend.get_custom_header(5);
    assert!(header.is_some(), "Custom header should not have been consumed");
    assert_eq!(header.unwrap().block_n, 5);
}

#[test]
fn custom_header_cleared_after_successful_validation() {
    let chain_config: Arc<ChainConfig> = ChainConfig::madara_test().into();
    let known_hash = discover_block_hash(chain_config.clone(), 0);

    let backend = MadaraBackend::open_for_testing(chain_config);
    backend
        .set_custom_header(CustomHeader { block_n: 0, expected_block_hash: known_hash, ..Default::default() })
        .expect("custom header should be staged");

    let _ =
        backend.write_access().add_full_block_with_classes(&test_block(0), &[], false).expect("Block should succeed");

    assert!(backend.get_custom_header(0).is_none(), "Custom header should be cleared after successful use");
}

#[test]
fn custom_header_lifecycle_matches_storage_contract() {
    let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));
    let initial = sample_custom_header(7, 1);
    let replacement = sample_custom_header(7, 2);

    backend.set_custom_header(initial.clone()).unwrap();

    let first = backend.get_custom_header(initial.block_n).expect("custom header should be present");
    let second = backend.get_custom_header(initial.block_n).expect("custom header should still be present");

    assert_custom_header_eq(&first, &initial);
    assert_custom_header_eq(&second, &initial);

    backend.set_custom_header(replacement.clone()).unwrap();

    let stored = backend.get_custom_header(replacement.block_n).expect("latest header should be stored");
    assert_custom_header_eq(&stored, &replacement);

    let taken = backend.take_custom_header(replacement.block_n).expect("take should return latest header");
    assert_custom_header_eq(&taken, &replacement);
    assert!(backend.get_custom_header(replacement.block_n).is_none(), "take should clear the overwritten entry");
    assert!(backend.take_custom_header(replacement.block_n).is_none(), "entry should only be consumed once");
}

#[test]
fn custom_header_does_not_mutate_live_preconfirmed_header_when_block_is_already_open() {
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
    backend.write_access().new_preconfirmed(PreconfirmedBlock::new(original_header.clone())).unwrap();

    let replacement = sample_custom_header(0, 9);
    backend.set_custom_header(replacement.clone()).unwrap();

    let preconfirmed = backend.block_view_on_preconfirmed().expect("live preconfirmed block should exist");
    assert_eq!(preconfirmed.block().header.block_number, replacement.block_n);
    assert_eq!(preconfirmed.block().header.block_timestamp.0, original_header.block_timestamp.0);
    assert_eq!(preconfirmed.block().header.gas_prices, original_header.gas_prices);

    let stored = backend.get_custom_header(replacement.block_n).expect("custom header should remain staged");
    assert_custom_header_eq(&stored, &replacement);
}

#[test]
fn clear_custom_headers_through_prunes_stale_entries() {
    let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));
    let header_3 = sample_custom_header(3, 1);
    let header_4 = sample_custom_header(4, 2);
    let header_5 = sample_custom_header(5, 3);

    backend.set_custom_header(header_3.clone()).unwrap();
    backend.set_custom_header(header_4.clone()).unwrap();
    backend.set_custom_header(header_5.clone()).unwrap();

    assert_eq!(backend.clear_custom_headers_through(4), 2);
    assert!(backend.get_custom_header(3).is_none(), "older header should be pruned");
    assert!(backend.get_custom_header(4).is_none(), "closed block header should be pruned");

    let remaining = backend.get_custom_header(5).expect("newer header should remain staged");
    assert_custom_header_eq(&remaining, &header_5);

    assert_eq!(backend.clear_custom_headers_through(5), 1);
    assert!(backend.get_custom_header(5).is_none(), "pruning through latest staged header should clear it");
}

#[test]
fn no_persistence_on_hash_mismatch() {
    let backend = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());

    assert!(backend.latest_confirmed_block_n().is_none(), "DB should start empty");

    backend
        .set_custom_header(CustomHeader {
            block_n: 0,
            expected_block_hash: Felt::from_hex_unchecked("0xdeadbeef"),
            ..Default::default()
        })
        .expect("custom header should be staged");

    let result = backend.write_access().add_full_block_with_classes(&test_block(0), &[], false);
    assert!(result.is_err());

    assert!(backend.latest_confirmed_block_n().is_none(), "No block should be persisted after hash mismatch");
}

#[test]
fn hash_mismatch_no_persistence_survives_restart() {
    let chain_config: Arc<ChainConfig> = ChainConfig::madara_test().into();

    let temp_dir = tempfile::TempDir::with_prefix("madara-test").unwrap();
    let cairo_native_config = {
        let builder = mc_class_exec::config::NativeConfig::builder();
        Arc::new(builder.build())
    };

    // First open: set wrong hash, try to add block 0 — should fail
    {
        let backend = MadaraBackend::open_rocksdb(
            temp_dir.path(),
            chain_config.clone(),
            MadaraBackendConfig { save_preconfirmed: false, ..Default::default() },
            RocksDBConfig::default(),
            cairo_native_config.clone(),
        )
        .expect("DB open should succeed");

        backend
            .set_custom_header(CustomHeader {
                block_n: 0,
                expected_block_hash: Felt::from_hex_unchecked("0xdeadbeef"),
                ..Default::default()
            })
            .expect("custom header should be staged");

        let result = backend.write_access().add_full_block_with_classes(&test_block(0), &[], false);
        assert!(result.is_err(), "Block add should fail on hash mismatch");
        assert!(backend.latest_confirmed_block_n().is_none(), "No block should be persisted after hash mismatch");

        backend.flush().expect("Flush should succeed");
    }
    // Backend dropped — simulates process restart

    // Second open: verify block 0 was NOT persisted
    {
        let backend = MadaraBackend::open_rocksdb(
            temp_dir.path(),
            chain_config,
            MadaraBackendConfig { save_preconfirmed: false, ..Default::default() },
            RocksDBConfig::default(),
            cairo_native_config,
        )
        .expect("DB reopen should succeed");

        assert!(
            backend.latest_confirmed_block_n().is_none(),
            "After restart, block 0 should not exist — staged tries were never committed"
        );
    }
}
