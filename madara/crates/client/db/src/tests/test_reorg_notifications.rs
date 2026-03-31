#![cfg(test)]

use crate::{test_utils::add_test_block, ChainTip, MadaraBackend, MadaraStorageRead, ReorgHead, ReorgNotification};
use mp_chain_config::ChainConfig;
use std::{sync::Arc, time::Duration};

#[tokio::test]
async fn revert_refreshes_chain_tip_cache_and_emits_reorg_notification() {
    let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));

    let block_0_hash = add_test_block(&backend, 0, vec![]);
    let block_1_hash = add_test_block(&backend, 1, vec![]);
    let block_2_hash = add_test_block(&backend, 2, vec![]);

    let mut chain_tip_watch = backend.watch_chain_tip();
    let mut reorgs = backend.subscribe_reorgs();

    assert_eq!(chain_tip_watch.current(), &ChainTip::Confirmed(2));
    assert_eq!(backend.latest_confirmed_block_n(), Some(2));

    let (new_tip_n, new_tip_hash) = backend.revert_to(&block_0_hash).expect("Revert should succeed");

    assert_eq!(new_tip_n, 0);
    assert_eq!(new_tip_hash, block_0_hash);
    assert_eq!(backend.latest_confirmed_block_n(), Some(0));
    assert_eq!(&*backend.chain_tip.borrow(), &ChainTip::Confirmed(0));

    let updated_tip = tokio::time::timeout(Duration::from_secs(1), chain_tip_watch.recv())
        .await
        .expect("Chain tip watcher should observe the revert");
    assert!(matches!(updated_tip, ChainTip::Confirmed(0)));

    let notification = tokio::time::timeout(Duration::from_secs(1), reorgs.recv())
        .await
        .expect("Reorg subscriber should receive a notification")
        .expect("Reorg receiver should stay connected");

    assert_eq!(
        notification,
        ReorgNotification {
            previous_head: ReorgHead {
                tip: ChainTip::Confirmed(2),
                latest_confirmed_block_n: 2,
                latest_confirmed_block_hash: block_2_hash,
            },
            new_head: ReorgHead {
                tip: ChainTip::Confirmed(0),
                latest_confirmed_block_n: 0,
                latest_confirmed_block_hash: block_0_hash,
            },
            first_reverted_block_n: 1,
            first_reverted_block_hash: block_1_hash,
        }
    );
    assert_eq!(notification.last_reverted_block_n(), 2);
    assert_eq!(notification.reverted_block_count(), 2);
}

#[tokio::test]
async fn revert_clamps_latest_l1_confirmed_cache_and_persisted_tip() {
    let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));

    let block_0_hash = add_test_block(&backend, 0, vec![]);
    add_test_block(&backend, 1, vec![]);
    add_test_block(&backend, 2, vec![]);
    backend.set_latest_l1_confirmed(Some(2)).expect("Setting latest L1-confirmed head should succeed");

    let mut l1_confirmed_watch = backend.watch_l1_confirmed();
    assert_eq!(l1_confirmed_watch.current(), &Some(2));

    backend.revert_to(&block_0_hash).expect("Revert should succeed");

    assert_eq!(backend.latest_l1_confirmed_block_n(), Some(0));
    assert_eq!(backend.db.get_confirmed_on_l1_tip().expect("DB read should succeed"), Some(0));

    let updated_l1_tip = tokio::time::timeout(Duration::from_secs(1), l1_confirmed_watch.recv())
        .await
        .expect("L1-confirmed watcher should observe the clamped tip");
    assert_eq!(updated_l1_tip, &Some(0));
}

#[tokio::test]
async fn revert_to_current_tip_does_not_emit_reorg_notification() {
    let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));

    let block_0_hash = add_test_block(&backend, 0, vec![]);
    let mut reorgs = backend.subscribe_reorgs();

    backend.revert_to(&block_0_hash).expect("Reverting to the current tip should succeed");

    let recv = tokio::time::timeout(Duration::from_millis(100), reorgs.recv()).await;
    assert!(recv.is_err(), "No reorg notification should be emitted when the tip is unchanged");
    assert_eq!(backend.latest_confirmed_block_n(), Some(0));
}
