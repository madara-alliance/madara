use super::{
    normalize_sender_address_filter, resolve_live_confirmed_head, LiveConfirmedHeadResolution, WsSubscriptionHandle,
};
use crate::{errors::StarknetWsApiError, test_utils::rpc_test_setup};
use mp_block::{header::PreconfirmedHeader, FullBlockWithoutCommitments};
use starknet_types_core::felt::Felt;
use std::{collections::HashSet, sync::Arc};

fn add_block_at(backend: &Arc<mc_db::MadaraBackend>, n: u64) -> Felt {
    backend
        .write_access()
        .add_full_block_with_classes(
            &FullBlockWithoutCommitments {
                header: PreconfirmedHeader { block_number: n, ..Default::default() },
                state_diff: mp_state_update::StateDiff::default(),
                transactions: vec![],
                events: vec![],
            },
            &[],
            false,
        )
        .expect("Storing block")
        .block_hash
}

#[test]
fn resolve_live_confirmed_head_returns_pending_reorg_before_reading_db() {
    let (backend, _rpc) = rpc_test_setup();
    let block_0_hash = add_block_at(&backend, 0);
    let block_1_hash = add_block_at(&backend, 1);
    let mut reorgs = backend.subscribe_reorgs();

    backend.revert_to(&block_0_hash).expect("Revert should succeed");

    match resolve_live_confirmed_head(&backend, &mut reorgs, 1, || StarknetWsApiError::Internal)
        .expect("Reorg resolution should succeed")
    {
        LiveConfirmedHeadResolution::Reorg(reorg) => {
            assert_eq!(reorg.first_reverted_block_n, 1);
            assert_eq!(reorg.first_reverted_block_hash, block_1_hash);
        }
        LiveConfirmedHeadResolution::Block(_) => panic!("Expected queued reorg before block read"),
        LiveConfirmedHeadResolution::RetryBackfill => panic!("Expected queued reorg before backfill retry"),
    }
}

#[test]
fn resolve_live_confirmed_head_retries_backfill_when_block_is_missing() {
    let (backend, _rpc) = rpc_test_setup();
    let mut reorgs = backend.subscribe_reorgs();

    match resolve_live_confirmed_head(&backend, &mut reorgs, 0, || StarknetWsApiError::Internal)
        .expect("Missing block should not error")
    {
        LiveConfirmedHeadResolution::RetryBackfill => {}
        LiveConfirmedHeadResolution::Block(_) => panic!("Expected missing block to retry backfill"),
        LiveConfirmedHeadResolution::Reorg(_) => panic!("Expected missing block without reorg to retry backfill"),
    }
}

#[test]
fn normalize_sender_address_filter_treats_empty_as_unfiltered() {
    assert_eq!(normalize_sender_address_filter(None), None);
    assert_eq!(normalize_sender_address_filter(Some(vec![])), None);
    assert_eq!(
        normalize_sender_address_filter(Some(vec![Felt::ONE, Felt::ONE, Felt::TWO])),
        Some(HashSet::from([Felt::ONE, Felt::TWO]))
    );
}

#[tokio::test]
async fn ws_subscription_handle_cancel_wakes_all_waiters() {
    let (handle, _cancelled) = WsSubscriptionHandle::new();
    let handle = Arc::new(handle);
    let handle_1 = Arc::clone(&handle);
    let handle_2 = Arc::clone(&handle);

    let waiter_1 = tokio::spawn(async move { handle_1.cancelled().await });
    let waiter_2 = tokio::spawn(async move { handle_2.cancelled().await });

    tokio::task::yield_now().await;
    handle.cancel();

    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        waiter_1.await.expect("First waiter should complete");
        waiter_2.await.expect("Second waiter should complete");
    })
    .await
    .expect("Cancellation should wake all waiters");
}

#[tokio::test]
async fn ws_subscription_handle_cancelled_returns_immediately_after_cancel() {
    let (handle, _cancelled) = WsSubscriptionHandle::new();
    handle.cancel();

    tokio::time::timeout(std::time::Duration::from_secs(1), handle.cancelled())
        .await
        .expect("Cancelled handle should resolve immediately");
}
