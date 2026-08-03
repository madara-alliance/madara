use crate::{
    constants::MAX_EVENTS_KEYS,
    errors::{ErrorExtWs, StarknetWsApiError},
};
use anyhow::Context;
use mc_db::{subscription::SubscribeNewBlocksTag, EventFilter};
use mp_block::EventWithInfo;
use mp_rpc::v0_10_0::{BlockId, BlockTag, EmittedEvent, FinalityStatus, TxnFinalityStatus};
use starknet_types_core::felt::Felt;

use super::BLOCK_PAST_LIMIT;

pub async fn subscribe_events(
    starknet: &crate::Starknet,
    subscription_sink: jsonrpsee::PendingSubscriptionSink,
    from_address: Option<Felt>,
    keys: Option<Vec<Vec<Felt>>>,
    block_id: Option<BlockId>,
    finality_status: Option<FinalityStatus>,
) -> Result<(), StarknetWsApiError> {
    if let Err(err) = validate_keys(&keys) {
        subscription_sink.reject(err).await;
        return Ok(());
    }

    let sink = subscription_sink.accept().await.or_internal_server_error("Failed to establish websocket connection")?;
    let ctx = starknet.ws_handles.subscription_register(sink.subscription_id()).await;
    let requested_finality = finality_status.unwrap_or_default();
    let mut reorgs = starknet.backend.subscribe_reorgs();

    let mut next_block_n = starknet.backend.latest_confirmed_block_n().map_or(0, |block_n| block_n.saturating_add(1));

    if let Some(block_id) = block_id {
        if matches!(block_id, BlockId::Tag(BlockTag::PreConfirmed)) {
            return Err(StarknetWsApiError::Pending);
        }

        let view = match starknet.resolve_view_on(block_id) {
            Ok(view) => view,
            Err(crate::StarknetRpcApiError::BlockNotFound) => return Err(StarknetWsApiError::BlockNotFound),
            Err(crate::StarknetRpcApiError::NoBlocks) => return Err(StarknetWsApiError::NoBlocks),
            Err(err) => return Err(StarknetWsApiError::internal_server_error(err.to_string())),
        };
        let latest_block = starknet.backend.view_on_latest().latest_block_n().ok_or(StarknetWsApiError::NoBlocks)?;
        let block_n = view.latest_block_n().ok_or(StarknetWsApiError::NoBlocks)?;

        if block_n < latest_block.saturating_sub(BLOCK_PAST_LIMIT) {
            return Err(StarknetWsApiError::TooManyBlocksBack);
        }

        next_block_n = block_n;
    }

    'backfill: loop {
        let backfill_to_block_n = starknet.backend.view_on_latest().latest_block_n();

        while backfill_to_block_n.is_some_and(|end_block_n| next_block_n <= end_block_n) {
            if sink.is_closed() {
                return Ok(());
            }
            if ctx.is_cancelled() {
                return Err(crate::errors::StarknetWsApiError::Internal);
            }

            match reorgs.try_recv() {
                Ok(reorg) => {
                    super::send_reorg_notification(&sink, &reorg).await?;
                    next_block_n = reorg.first_reverted_block_n;
                    continue 'backfill;
                }
                Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => {
                    return Err(super::missed_reorg_notifications_error());
                }
                Err(tokio::sync::broadcast::error::TryRecvError::Closed) => {
                    return Err(crate::errors::StarknetWsApiError::Internal);
                }
                Err(tokio::sync::broadcast::error::TryRecvError::Empty) => {}
            }

            if ctx.is_cancelled() {
                return Err(crate::errors::StarknetWsApiError::Internal);
            }
            let mut live_reorgs = Some(&mut reorgs);
            if let Some(reorg) = send_block_events(
                starknet,
                &sink,
                &from_address,
                &keys,
                &mut live_reorgs,
                next_block_n,
                &requested_finality,
            )
            .await?
            {
                super::send_reorg_notification(&sink, &reorg).await?;
                next_block_n = reorg.first_reverted_block_n;
                continue 'backfill;
            }
            next_block_n = next_block_n.saturating_add(1);
        }

        let mut heads = starknet.backend.subscribe_new_heads(SubscribeNewBlocksTag::Preconfirmed);
        heads.set_start_from(next_block_n);

        loop {
            let block_view = tokio::select! {
                block_view = heads.next_block_view() => block_view,
                reorg = reorgs.recv() => {
                    match reorg {
                        Ok(reorg) => {
                            super::send_reorg_notification(&sink, &reorg).await?;
                            next_block_n = reorg.first_reverted_block_n;
                            continue 'backfill;
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                            return Err(super::missed_reorg_notifications_error());
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            return Err(crate::errors::StarknetWsApiError::Internal);
                        }
                    }
                },
                _ = sink.closed() => return Ok(()),
                _ = ctx.cancelled() => return Err(crate::errors::StarknetWsApiError::Internal),
            };

            let block_number = block_view.block_number();
            if block_view.is_confirmed() {
                match crate::resolve_live_confirmed_head(
                    &starknet.backend,
                    &mut reorgs,
                    block_number,
                    super::missed_reorg_notifications_error(),
                )? {
                    crate::LiveConfirmedHeadResolution::Block(_) => {}
                    crate::LiveConfirmedHeadResolution::Reorg(reorg) => {
                        super::send_reorg_notification(&sink, &reorg).await?;
                        next_block_n = reorg.first_reverted_block_n;
                        continue 'backfill;
                    }
                    crate::LiveConfirmedHeadResolution::RetryBackfill => {
                        next_block_n = block_number;
                        continue 'backfill;
                    }
                }
            }
            let mut live_reorgs = Some(&mut reorgs);
            if let Some(reorg) = send_block_events(
                starknet,
                &sink,
                &from_address,
                &keys,
                &mut live_reorgs,
                block_number,
                &requested_finality,
            )
            .await?
            {
                super::send_reorg_notification(&sink, &reorg).await?;
                next_block_n = reorg.first_reverted_block_n;
                continue 'backfill;
            }
        }
    }
}

fn validate_keys(keys: &Option<Vec<Vec<Felt>>>) -> Result<(), StarknetWsApiError> {
    let total_keys = keys.as_ref().map(|patterns| patterns.iter().map(Vec::len).sum::<usize>()).unwrap_or(0);
    if total_keys > MAX_EVENTS_KEYS {
        return Err(StarknetWsApiError::TooManyKeysInFilter);
    }

    Ok(())
}

async fn send_block_events(
    starknet: &crate::Starknet,
    sink: &jsonrpsee::core::server::SubscriptionSink,
    from_address: &Option<Felt>,
    keys: &Option<Vec<Vec<Felt>>>,
    reorgs: &mut Option<&mut mc_db::subscription::SubscribeReorgs<mc_db::rocksdb::RocksDBStorage>>,
    block_number: u64,
    requested_finality: &FinalityStatus,
) -> Result<Option<mc_db::ReorgNotification>, StarknetWsApiError> {
    let view = starknet.backend.view_on_latest();
    let latest_l1_confirmed_block_n = view.latest_l1_confirmed_block_n();

    let events = view
        .get_events(EventFilter {
            start_block: block_number,
            start_event_index: 0,
            end_block: block_number,
            from_address: *from_address,
            keys_pattern: keys.clone(),
            max_events: usize::MAX,
        })
        .context("Error getting filtered events")
        .or_internal_server_error("Failed to retrieve filtered events")?;

    if let Some(reorg) = take_pending_reorg(reorgs)? {
        return Ok(Some(reorg));
    }

    for event in events {
        if let Some(reorg) = take_pending_reorg(reorgs)? {
            return Ok(Some(reorg));
        }
        let finality_status = event_finality_status(&event, latest_l1_confirmed_block_n);
        if !subscription_allows_finality(requested_finality, finality_status) {
            continue;
        }

        send_event(event, finality_status, sink).await?;
    }

    Ok(None)
}

fn take_pending_reorg(
    reorgs: &mut Option<&mut mc_db::subscription::SubscribeReorgs<mc_db::rocksdb::RocksDBStorage>>,
) -> Result<Option<mc_db::ReorgNotification>, StarknetWsApiError> {
    match reorgs.as_deref_mut() {
        Some(reorgs) => crate::try_recv_live_reorg(reorgs, super::missed_reorg_notifications_error()),
        None => Ok(None),
    }
}

fn subscription_allows_finality(requested_finality: &FinalityStatus, event_finality: TxnFinalityStatus) -> bool {
    match requested_finality {
        FinalityStatus::PreConfirmed => matches!(event_finality, TxnFinalityStatus::PreConfirmed),
        FinalityStatus::AcceptedOnL2 => !matches!(event_finality, TxnFinalityStatus::PreConfirmed),
    }
}

fn event_finality_status(event: &EventWithInfo, latest_l1_confirmed_block_n: Option<u64>) -> TxnFinalityStatus {
    if event.in_preconfirmed {
        return TxnFinalityStatus::PreConfirmed;
    }

    if latest_l1_confirmed_block_n.is_some_and(|last_on_l1| event.block_number <= last_on_l1) {
        TxnFinalityStatus::L1
    } else {
        TxnFinalityStatus::L2
    }
}

async fn send_event(
    event: EventWithInfo,
    finality_status: TxnFinalityStatus,
    sink: &jsonrpsee::core::server::SubscriptionSink,
) -> Result<(), StarknetWsApiError> {
    let emitted_event = EmittedEvent::from(event);
    let item = super::SubscriptionItem::new(
        sink.subscription_id(),
        mp_rpc::v0_10_0::EmittedEventWithFinality { emitted_event, finality_status },
    );
    let msg = jsonrpsee::SubscriptionMessage::from_json(&item)
        .or_internal_server_error("Failed to create response message")?;
    sink.send(msg).await.or_internal_server_error("Failed to respond to websocket request")
}
