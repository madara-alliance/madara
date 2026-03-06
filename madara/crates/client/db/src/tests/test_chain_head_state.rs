#![cfg(test)]

use crate::{
    chain_head::ChainHeadState, metrics, preconfirmed::PreconfirmedBlock, storage::StorageHeadProjection,
    test_utils::add_test_block, MadaraBackend, MadaraBackendConfig, MadaraBlockView, MadaraStorageRead,
};
use mp_block::{header::PreconfirmedHeader, FullBlockWithoutCommitments};
use mp_chain_config::ChainConfig;
use mp_state_update::StateDiff;
use rstest::rstest;
use std::sync::Arc;

#[tokio::test]
async fn internal_heads_subscription_tracks_preconfirmed_then_confirmed_close() {
    let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));
    let mut sub = backend.subscribe_internal_heads(crate::subscription::SubscribeNewBlocksTag::Preconfirmed);
    sub.set_start_from(0);

    backend
        .write_access()
        .new_preconfirmed(PreconfirmedBlock::new(PreconfirmedHeader { block_number: 0, ..Default::default() }))
        .expect("creating preconfirmed block should succeed");

    let preconfirmed_head = sub.next_block_view().await;
    match preconfirmed_head {
        MadaraBlockView::Preconfirmed(block) => assert_eq!(block.block_number(), 0),
        _ => panic!("expected preconfirmed head"),
    }

    let head = backend.chain_head_state();
    assert_eq!(head.confirmed_tip, None);
    assert_eq!(head.external_preconfirmed_tip, Some(0));
    assert_eq!(head.internal_preconfirmed_tip, Some(0));

    backend
        .write_access()
        .add_full_block_with_classes(
            &FullBlockWithoutCommitments {
                header: PreconfirmedHeader { block_number: 0, ..Default::default() },
                state_diff: StateDiff::default(),
                transactions: vec![],
                events: vec![],
            },
            &[],
            false,
        )
        .expect("closing block should succeed");

    let confirmed_head = sub.next_block_view().await;
    match confirmed_head {
        MadaraBlockView::Confirmed(block) => assert_eq!(block.block_number(), 0),
        _ => panic!("expected confirmed head"),
    }

    let head = backend.chain_head_state();
    assert_eq!(head.confirmed_tip, Some(0));
    assert_eq!(head.external_preconfirmed_tip, None);
    assert_eq!(head.internal_preconfirmed_tip, None);
}

#[tokio::test]
async fn internal_heads_subscription_emits_internal_runahead_while_external_tip_is_stable() {
    let backend = MadaraBackend::open_for_testing_with_config(
        Arc::new(ChainConfig::madara_test()),
        MadaraBackendConfig { save_preconfirmed: true, ..Default::default() },
    );
    let mut sub = backend.subscribe_internal_heads(crate::subscription::SubscribeNewBlocksTag::Preconfirmed);
    sub.set_start_from(0);

    backend
        .write_access()
        .new_preconfirmed(PreconfirmedBlock::new(PreconfirmedHeader { block_number: 0, ..Default::default() }))
        .expect("creating preconfirmed block #0 should succeed");

    let first = sub.next_block_view().await;
    match first {
        MadaraBlockView::Preconfirmed(block) => assert_eq!(block.block_number(), 0),
        _ => panic!("expected preconfirmed head"),
    }

    backend
        .write_access()
        .new_preconfirmed(PreconfirmedBlock::new(PreconfirmedHeader { block_number: 1, ..Default::default() }))
        .expect("creating internal runahead preconfirmed block #1 should succeed");

    let head = backend.chain_head_state();
    assert_eq!(head.external_preconfirmed_tip, Some(0));
    assert_eq!(head.internal_preconfirmed_tip, Some(1));

    let second = sub.next_block_view().await;
    match second {
        MadaraBlockView::Preconfirmed(block) => assert_eq!(block.block_number(), 1),
        _ => panic!("expected preconfirmed head"),
    }
}

#[test]
fn runahead_keeps_external_visibility_and_promotes_next_on_confirm() {
    let backend = MadaraBackend::open_for_testing_with_config(
        Arc::new(ChainConfig::madara_test()),
        MadaraBackendConfig { save_preconfirmed: true, ..Default::default() },
    );

    backend
        .write_access()
        .new_preconfirmed(PreconfirmedBlock::new(PreconfirmedHeader { block_number: 0, ..Default::default() }))
        .expect("creating preconfirmed block #0 should succeed");
    backend
        .write_access()
        .new_preconfirmed(PreconfirmedBlock::new(PreconfirmedHeader { block_number: 1, ..Default::default() }))
        .expect("creating internal runahead preconfirmed block #1 should succeed");

    let head = backend.chain_head_state();
    assert_eq!(head.confirmed_tip, None);
    assert_eq!(head.external_preconfirmed_tip, Some(0));
    assert_eq!(head.internal_preconfirmed_tip, Some(1));
    assert_eq!(
        backend.db.get_latest_preconfirmed_header_block_n().expect("reading latest preconfirmed header tip"),
        Some(1)
    );

    match backend.db.get_head_projection().expect("reading head projection") {
        StorageHeadProjection::Preconfirmed { header, .. } => assert_eq!(header.block_number, 0),
        other => panic!("expected preconfirmed head projection, got {other:?}"),
    }

    assert_eq!(
        backend.block_view_on_current_preconfirmed().expect("external preconfirmed view should exist").block_number(),
        0
    );
    assert_eq!(
        backend
            .block_view_on_preconfirmed(1)
            .expect("internal runahead preconfirmed block should be queryable")
            .block_number(),
        1
    );

    backend.write_access().new_confirmed_block(0).expect("confirming external preconfirmed block should succeed");

    let head = backend.chain_head_state();
    assert_eq!(head.confirmed_tip, Some(0));
    assert_eq!(head.external_preconfirmed_tip, Some(1));
    assert_eq!(head.internal_preconfirmed_tip, Some(1));

    match backend.db.get_head_projection().expect("reading head projection after confirmation") {
        StorageHeadProjection::Preconfirmed { header, .. } => assert_eq!(header.block_number, 1),
        other => panic!("expected promoted preconfirmed head projection, got {other:?}"),
    }
}

#[test]
fn refresh_reconstructs_internal_tip_from_persisted_preconfirmed_headers() {
    let backend = MadaraBackend::open_for_testing_with_config(
        Arc::new(ChainConfig::madara_test()),
        MadaraBackendConfig { save_preconfirmed: true, ..Default::default() },
    );

    for block_n in 0..=2 {
        backend
            .write_access()
            .new_preconfirmed(PreconfirmedBlock::new(PreconfirmedHeader {
                block_number: block_n,
                ..Default::default()
            }))
            .expect("creating preconfirmed block should succeed");
    }

    let stale_runtime_block =
        backend.block_view_on_preconfirmed(0).expect("preconfirmed block #0 must exist").block().clone();
    backend
        .publish_head_projection(
            ChainHeadState {
                confirmed_tip: None,
                external_preconfirmed_tip: Some(0),
                internal_preconfirmed_tip: Some(0),
            },
            Some(stale_runtime_block),
        )
        .expect("publishing stale in-memory projection should succeed");

    assert_eq!(backend.chain_head_state().internal_preconfirmed_tip, Some(0));

    backend.refresh_head_projection_from_db().expect("refresh should succeed");

    let refreshed_head = backend.chain_head_state();
    assert_eq!(refreshed_head.confirmed_tip, None);
    assert_eq!(refreshed_head.external_preconfirmed_tip, Some(0));
    assert_eq!(refreshed_head.internal_preconfirmed_tip, Some(2));
    assert_eq!(
        backend
            .internal_preconfirmed_block()
            .expect("runtime preconfirmed should align to internal tip")
            .header
            .block_number,
        2
    );
}

#[rstest]
#[case::lagging_tip(9, true)]
#[case::equal_tip(10, true)]
#[case::ahead_tip(11, false)]
fn confirmed_projection_monotonicity(#[case] projected_tip: u64, #[case] expect_ok: bool) {
    let head =
        ChainHeadState { confirmed_tip: Some(10), external_preconfirmed_tip: None, internal_preconfirmed_tip: None };
    let result = MadaraBackend::<crate::rocksdb::RocksDBStorage>::ensure_tip_not_ahead_of_head_state(
        head,
        &StorageHeadProjection::Confirmed(projected_tip),
    );
    assert_eq!(result.is_ok(), expect_ok, "unexpected monotonicity result for tip={projected_tip}");
}

#[test]
fn projected_preconfirmed_mismatch_is_rejected() {
    let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));

    let preconfirmed = Arc::new(PreconfirmedBlock::new(PreconfirmedHeader { block_number: 12, ..Default::default() }));
    let head = ChainHeadState {
        confirmed_tip: Some(10),
        external_preconfirmed_tip: Some(11),
        internal_preconfirmed_tip: Some(11),
    };

    let result = backend.publish_head_projection(head, Some(preconfirmed));
    assert!(result.is_err(), "preconfirmed runtime/head-state mismatch must be rejected");
}

/// Matrix: preconfirmed projection matching/mismatching cases.
#[rstest]
#[case::matching_preconfirmed(Some(10), Some(11), 11, true)]
#[case::mismatched_preconfirmed(Some(10), Some(11), 12, false)]
#[case::no_head_preconfirmed_but_projected(Some(10), None, 11, false)]
fn preconfirmed_projection_matrix(
    #[case] confirmed_tip: Option<u64>,
    #[case] external_preconfirmed_tip: Option<u64>,
    #[case] projected_preconfirmed_block_n: u64,
    #[case] expect_ok: bool,
) {
    let head = ChainHeadState {
        confirmed_tip,
        external_preconfirmed_tip,
        internal_preconfirmed_tip: external_preconfirmed_tip,
    };
    let projection = StorageHeadProjection::Preconfirmed {
        header: PreconfirmedHeader { block_number: projected_preconfirmed_block_n, ..Default::default() },
        content: vec![],
    };
    let result = MadaraBackend::<crate::rocksdb::RocksDBStorage>::ensure_tip_not_ahead_of_head_state(head, &projection);
    assert_eq!(
        result.is_ok(),
        expect_ok,
        "unexpected preconfirmed projection result for block_n={projected_preconfirmed_block_n}"
    );
}

/// Revert + refresh reconstructs valid head state without drift.
#[test]
fn revert_refresh_reconstruction() {
    let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));

    // Add two confirmed blocks: block 0 and block 1.
    let block_0_hash = add_test_block(&backend, 0, vec![]);
    let _block_1_hash = add_test_block(&backend, 1, vec![]);

    // Verify head is at confirmed=1.
    let head = backend.chain_head_state();
    assert_eq!(head.confirmed_tip, Some(1));
    assert_eq!(head.external_preconfirmed_tip, None);

    // Revert to block 0 (DB-level revert).
    let (new_tip_n, new_tip_hash) = backend.revert_to(&block_0_hash).expect("revert should succeed");
    assert_eq!(new_tip_n, 0);
    assert_eq!(new_tip_hash, block_0_hash);

    // DB projection should be updated but in-memory head is stale.
    assert!(matches!(backend.db.get_head_projection().expect("db read"), StorageHeadProjection::Confirmed(0)));

    // Refresh in-memory head from DB.
    backend.refresh_head_projection_from_db().expect("refresh should succeed");

    // Verify head is reconstructed to confirmed=0, no preconfirmed.
    let head = backend.chain_head_state();
    assert_eq!(head.confirmed_tip, Some(0));
    assert_eq!(head.external_preconfirmed_tip, None);
    assert_eq!(head.internal_preconfirmed_tip, None);

    // Cross-field invariants must hold on reconstructed head.
    head.validate_cross_field_invariants().expect("cross-field invariants must hold after revert+refresh");
}

#[test]
fn revert_floor_replay_rewinds_and_reanchors_checkpoint_metadata() {
    let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));

    let mut block_hashes = Vec::new();
    for block_n in 0..=5 {
        block_hashes.push(add_test_block(&backend, block_n, vec![]));
    }

    backend.write_parallel_merkle_checkpoint(2).expect("checkpoint 2");
    backend.write_parallel_merkle_checkpoint(5).expect("checkpoint 5");
    assert_eq!(backend.get_parallel_merkle_latest_checkpoint().expect("latest checkpoint"), Some(5));

    let target_block_n = 4usize;
    let target_hash = block_hashes[target_block_n];
    let (new_tip_n, new_tip_hash) = backend.revert_to(&target_hash).expect("revert to block 4 should succeed");
    assert_eq!(new_tip_n, target_block_n as u64);
    assert_eq!(new_tip_hash, target_hash);

    // Floor+replay mode should prune future checkpoint metadata and re-anchor at target.
    assert_eq!(backend.get_parallel_merkle_latest_checkpoint().expect("latest checkpoint"), Some(4));
    assert!(backend.has_parallel_merkle_checkpoint(2).expect("checkpoint 2 should remain"));
    assert!(backend.has_parallel_merkle_checkpoint(4).expect("target checkpoint should be materialized"));
    assert!(!backend.has_parallel_merkle_checkpoint(5).expect("checkpoint above target should be removed"));
}

#[test]
fn revert_errors_when_latest_checkpoint_exists_but_no_floor_covers_target() {
    let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));

    let mut block_hashes = Vec::new();
    for block_n in 0..=5 {
        block_hashes.push(add_test_block(&backend, block_n, vec![]));
    }

    backend.write_parallel_merkle_checkpoint(4).expect("checkpoint 4");
    backend.write_parallel_merkle_checkpoint(5).expect("checkpoint 5");

    let err =
        backend.revert_to(&block_hashes[3]).expect_err("revert should fail when no checkpoint floor covers the target");

    assert!(
        err.to_string()
            .contains("Missing parallel merkle checkpoint floor for revert target 3 with latest checkpoint 5"),
        "unexpected error: {err:#}"
    );
    assert_eq!(backend.get_parallel_merkle_latest_checkpoint().expect("latest checkpoint"), Some(5));
    assert!(backend.has_parallel_merkle_checkpoint(4).expect("checkpoint 4 should remain"));
    assert!(backend.has_parallel_merkle_checkpoint(5).expect("checkpoint 5 should remain"));
}

/// Metric violation counter increments on induced projection mismatch.
#[test]
fn metric_violation_counter_increments() {
    use std::sync::atomic::Ordering;

    let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));
    let test_counter = &metrics::metrics().head_projection_violation_count_test;

    // Record baseline counter value.
    let baseline = test_counter.load(Ordering::Relaxed);

    // 1. Induce a projection violation: projected confirmed tip ahead of head.
    let head =
        ChainHeadState { confirmed_tip: Some(10), external_preconfirmed_tip: None, internal_preconfirmed_tip: None };
    let ahead_projection = StorageHeadProjection::Confirmed(11);
    let result =
        MadaraBackend::<crate::rocksdb::RocksDBStorage>::ensure_tip_not_ahead_of_head_state(head, &ahead_projection);
    assert!(result.is_err(), "ahead projection must be rejected");
    assert_eq!(
        test_counter.load(Ordering::Relaxed),
        baseline + 1,
        "violation counter must increment by 1 after ahead-projection rejection"
    );

    // 2. Induce a runtime preconfirmed mismatch via publish_head_projection.
    let mismatched_preconfirmed =
        Arc::new(PreconfirmedBlock::new(PreconfirmedHeader { block_number: 99, ..Default::default() }));
    let head_with_preconfirmed = ChainHeadState {
        confirmed_tip: Some(10),
        external_preconfirmed_tip: Some(11),
        internal_preconfirmed_tip: Some(11),
    };
    let result = backend.publish_head_projection(head_with_preconfirmed, Some(mismatched_preconfirmed));
    assert!(result.is_err(), "runtime preconfirmed mismatch must be rejected");
    assert_eq!(
        test_counter.load(Ordering::Relaxed),
        baseline + 2,
        "violation counter must increment by 1 after runtime-preconfirmed mismatch"
    );

    // 3. Induce a cross-field invariant violation via publish_head_projection.
    let bad_cross_field = ChainHeadState {
        confirmed_tip: Some(10),
        external_preconfirmed_tip: Some(10), // invalid: confirmed not strictly less than external
        internal_preconfirmed_tip: Some(10),
    };
    let result = backend.publish_head_projection(bad_cross_field, None);
    assert!(result.is_err(), "cross-field invariant violation must be rejected");
    assert_eq!(
        test_counter.load(Ordering::Relaxed),
        baseline + 3,
        "violation counter must increment by 1 after cross-field invariant violation"
    );
}
