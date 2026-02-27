#![cfg(test)]

use crate::{preconfirmed::PreconfirmedBlock, MadaraBackend, MadaraBlockView};
use mp_block::{header::PreconfirmedHeader, FullBlockWithoutCommitments};
use mp_chain_config::ChainConfig;
use mp_state_update::StateDiff;
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
