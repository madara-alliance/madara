#![cfg(test)]
use crate::rocksdb::RocksDBConfig;
use crate::{MadaraBackend, MadaraBackendConfig};
use mp_block::header::{CustomHeader, PreconfirmedHeader};
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
    backend.set_custom_header(CustomHeader { block_n: 0, expected_block_hash: known_hash, ..Default::default() });

    let result = backend.write_access().add_full_block_with_classes(&test_block(0), &[], false);

    assert!(result.is_ok());
    assert_eq!(result.unwrap().block_hash, known_hash);
    assert_eq!(backend.latest_confirmed_block_n(), Some(0));
}

#[test]
fn custom_header_hash_mismatch_returns_error() {
    let backend = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());
    backend.set_custom_header(CustomHeader {
        block_n: 0,
        expected_block_hash: Felt::from_hex_unchecked("0xdeadbeef"),
        ..Default::default()
    });

    let result = backend.write_access().add_full_block_with_classes(&test_block(0), &[], false);

    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("Block hash mismatch"), "Expected hash mismatch error, got: {err_msg}");
}

#[test]
fn custom_header_block_number_mismatch_returns_error() {
    let backend = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());
    backend.set_custom_header(CustomHeader {
        block_n: 5,
        expected_block_hash: Felt::from_hex_unchecked("0x1234"),
        ..Default::default()
    });

    let result = backend.write_access().add_full_block_with_classes(&test_block(0), &[], false);

    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("block number mismatch"), "Expected block number mismatch error, got: {err_msg}");
}

#[test]
fn custom_header_not_consumed_on_block_n_mismatch() {
    let backend = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());
    backend.set_custom_header(CustomHeader {
        block_n: 5,
        expected_block_hash: Felt::from_hex_unchecked("0x1234"),
        ..Default::default()
    });

    let _ = backend.write_access().add_full_block_with_classes(&test_block(0), &[], false);

    let header = backend.get_custom_header();
    assert!(header.is_some(), "Custom header should not have been consumed");
    assert_eq!(header.unwrap().block_n, 5);
}

#[test]
fn custom_header_cleared_after_successful_validation() {
    let chain_config: Arc<ChainConfig> = ChainConfig::madara_test().into();
    let known_hash = discover_block_hash(chain_config.clone(), 0);

    let backend = MadaraBackend::open_for_testing(chain_config);
    backend.set_custom_header(CustomHeader { block_n: 0, expected_block_hash: known_hash, ..Default::default() });

    let _ =
        backend.write_access().add_full_block_with_classes(&test_block(0), &[], false).expect("Block should succeed");

    assert!(backend.get_custom_header().is_none(), "Custom header should be cleared after successful use");
}

#[test]
fn no_persistence_on_hash_mismatch() {
    let backend = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());

    assert!(backend.latest_confirmed_block_n().is_none(), "DB should start empty");

    backend.set_custom_header(CustomHeader {
        block_n: 0,
        expected_block_hash: Felt::from_hex_unchecked("0xdeadbeef"),
        ..Default::default()
    });

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

        backend.set_custom_header(CustomHeader {
            block_n: 0,
            expected_block_hash: Felt::from_hex_unchecked("0xdeadbeef"),
            ..Default::default()
        });

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
