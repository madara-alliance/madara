use crate::errors::{ErrorExtWs, StarknetWsApiError};
use mc_db::subscription::SubscribeNewBlocksTag;
use mp_rpc::v0_10_2::{BlockHeader, BlockId, BlockTag};

use super::BLOCK_PAST_LIMIT;

pub async fn subscribe_new_heads(
    starknet: &crate::Starknet,
    subscription_sink: jsonrpsee::PendingSubscriptionSink,
    block_id: BlockId,
) -> Result<(), StarknetWsApiError> {
    let sink = subscription_sink.accept().await.or_internal_server_error("Failed to establish websocket connection")?;
    let ctx =
        starknet.ws_handles.subscription_register(sink.subscription_id(), crate::metrics::SUBSCRIBE_NEW_HEADS).await;

    let mut block_n = match block_id {
        BlockId::Number(block_n) => {
            let block_latest = starknet.backend.latest_confirmed_block_n().ok_or(StarknetWsApiError::NoBlocks)?;

            if block_n < block_latest.saturating_sub(BLOCK_PAST_LIMIT) {
                return Err(StarknetWsApiError::TooManyBlocksBack);
            }

            block_n
        }
        BlockId::Hash(block_hash) => {
            let block_latest = starknet.backend.latest_confirmed_block_n().ok_or(StarknetWsApiError::NoBlocks)?;
            let block_n = starknet
                .backend
                .view_on_latest_confirmed()
                .find_block_by_hash(&block_hash)
                .or_else_internal_server_error(|| format!("Failed to retrieve block info at hash {block_hash:#x}"))?
                .ok_or(StarknetWsApiError::BlockNotFound)?;

            if block_n < block_latest.saturating_sub(BLOCK_PAST_LIMIT) {
                return Err(StarknetWsApiError::TooManyBlocksBack);
            }

            block_n
        }
        BlockId::Tag(BlockTag::Latest) => {
            starknet.backend.latest_confirmed_block_n().ok_or(StarknetWsApiError::NoBlocks)?
        }
        BlockId::Tag(BlockTag::PreConfirmed) => {
            return Err(StarknetWsApiError::Pending);
        }
        BlockId::Tag(BlockTag::L1Accepted) => {
            starknet.backend.latest_l1_confirmed_block_n().ok_or(StarknetWsApiError::NoBlocks)?
        }
    };

    let mut reorgs = starknet.backend.subscribe_reorgs();

    'backfill: loop {
        loop {
            if sink.is_closed() {
                return Ok(());
            }
            if ctx.is_cancelled() {
                return Err(crate::errors::StarknetWsApiError::SubscriptionClosed);
            }

            match reorgs.try_recv() {
                Ok(reorg) => {
                    super::send_reorg_notification(&sink, &reorg).await?;
                    block_n = reorg.first_reverted_block_n;
                    continue 'backfill;
                }
                Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => {
                    crate::metrics::record_lagged_reorg(crate::metrics::SUBSCRIBE_NEW_HEADS);
                    return Err(super::missed_reorg_notifications_error());
                }
                Err(tokio::sync::broadcast::error::TryRecvError::Closed) => {
                    return Err(crate::errors::StarknetWsApiError::Internal);
                }
                Err(tokio::sync::broadcast::error::TryRecvError::Empty) => {}
            }

            let Some(block_view) = starknet.backend.block_view_on_confirmed(block_n) else {
                break;
            };
            let block_info = block_view
                .get_block_info()
                .or_else_internal_server_error(|| format!("Failed to retrieve block info for block {block_n}"))?;

            if block_info.header.block_number != block_n {
                let err = format!("Retrieved mismatched block {}, expected {block_n}", block_info.header.block_number);
                return Err(StarknetWsApiError::internal_server_error(err));
            }
            if ctx.is_cancelled() {
                return Err(crate::errors::StarknetWsApiError::SubscriptionClosed);
            }

            send_block_header(&sink, block_info, block_n).await?;
            block_n = block_n.saturating_add(1);
        }

        let mut heads = starknet.backend.subscribe_new_heads(SubscribeNewBlocksTag::Confirmed);
        heads.set_start_from(block_n);

        loop {
            let next_block_n = tokio::select! {
                head = heads.next_head() => head.latest_confirmed_block_n(),
                reorg = reorgs.recv() => {
                    match reorg {
                        Ok(reorg) => {
                            super::send_reorg_notification(&sink, &reorg).await?;
                            block_n = reorg.first_reverted_block_n;
                            continue 'backfill;
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                            crate::metrics::record_lagged_reorg(crate::metrics::SUBSCRIBE_NEW_HEADS);
                            return Err(super::missed_reorg_notifications_error());
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            return Err(crate::errors::StarknetWsApiError::Internal);
                        }
                    }
                },
                _ = sink.closed() => return Ok(()),
                _ = ctx.cancelled() => return Err(crate::errors::StarknetWsApiError::SubscriptionClosed),
            };

            let next_block_n =
                next_block_n.expect("Confirmed block subscription should always yield a confirmed block number");
            match crate::resolve_live_confirmed_head(
                &starknet.backend,
                &mut reorgs,
                next_block_n,
                super::missed_reorg_notifications_error,
            )? {
                crate::LiveConfirmedHeadResolution::Block(block_info) => {
                    send_block_header(&sink, *block_info, next_block_n).await?;
                }
                crate::LiveConfirmedHeadResolution::Reorg(reorg) => {
                    super::send_reorg_notification(&sink, &reorg).await?;
                    block_n = reorg.first_reverted_block_n;
                    continue 'backfill;
                }
                crate::LiveConfirmedHeadResolution::RetryBackfill => {
                    block_n = next_block_n;
                    continue 'backfill;
                }
            }
        }
    }
}

async fn send_block_header(
    sink: &jsonrpsee::core::server::SubscriptionSink,
    block_info: mp_block::MadaraBlockInfo,
    _block_n: u64,
) -> Result<(), StarknetWsApiError> {
    let header: BlockHeader = block_info.to_rpc_v0_10();
    super::send_starknet_subscription(sink, super::NEW_HEADS_NOTIFICATION_METHOD, &header).await
}
