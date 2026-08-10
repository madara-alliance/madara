#![cfg(test)]

use crate::MadaraBackend;
use mp_chain_config::ChainConfig;
use mp_rpc::admin::ReplayBlockBoundary;
use starknet_types_core::felt::Felt;

#[test]
fn replay_boundary_allows_boundary_hash_before_final_local_execution() {
    let backend = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());

    let first_hash = Felt::from_hex_unchecked("0x1");
    let boundary_hash = Felt::from_hex_unchecked("0x2");
    let final_hash = Felt::from_hex_unchecked("0x3");

    let status = backend.set_replay_boundary(ReplayBlockBoundary {
        block_n: 7,
        expected_tx_count: 3,
        last_tx_hash: boundary_hash,
    });
    assert_eq!(status.executed_tx_count, 0);
    assert!(!status.reached_last_tx_hash);
    assert!(!status.boundary_met);
    assert!(status.mismatch.is_none());

    let status = backend
        .replay_boundary_record_executed_hashes(7, &[first_hash, boundary_hash])
        .expect("replay boundary status");
    assert_eq!(status.executed_tx_count, 2);
    assert_eq!(status.last_executed_tx_hash, Some(boundary_hash));
    assert!(status.reached_last_tx_hash, "boundary hash should be remembered once observed");
    assert!(!status.boundary_met, "boundary should not be met before expected count is reached");
    assert!(status.mismatch.is_none(), "early observation of the boundary hash is valid for mixed-route blocks");

    let status = backend.replay_boundary_record_executed_hashes(7, &[final_hash]).expect("replay boundary status");
    assert_eq!(status.executed_tx_count, 3);
    assert_eq!(status.last_executed_tx_hash, Some(final_hash));
    assert!(status.reached_last_tx_hash);
    assert!(status.boundary_met, "boundary should be met once all expected txs execute and the boundary hash was seen");
    assert!(status.mismatch.is_none());
}

#[test]
fn replay_boundary_rejects_expected_count_without_boundary_hash() {
    let backend = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());

    let boundary_hash = Felt::from_hex_unchecked("0x20");
    let executed_hashes =
        [Felt::from_hex_unchecked("0x10"), Felt::from_hex_unchecked("0x11"), Felt::from_hex_unchecked("0x12")];

    backend.set_replay_boundary(ReplayBlockBoundary {
        block_n: 9,
        expected_tx_count: executed_hashes.len() as u64,
        last_tx_hash: boundary_hash,
    });

    let status = backend.replay_boundary_record_executed_hashes(9, &executed_hashes).expect("replay boundary status");
    assert_eq!(status.executed_tx_count, executed_hashes.len() as u64);
    assert!(!status.reached_last_tx_hash);
    assert!(!status.boundary_met);
    let mismatch = status.mismatch.expect("missing boundary hash should be reported as mismatch");
    assert!(mismatch.contains("boundary_last_tx_hash"), "unexpected mismatch message: {mismatch}");
}
