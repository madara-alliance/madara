use crate::constants::MAX_EVENTS_KEYS;
use crate::errors::{ErrorExtWs, StarknetWsApiError};
use anyhow::Context;
use mc_db::{subscription::SubscribeNewBlocksTag, EventFilter};
use mp_block::EventWithInfo;
use mp_rpc::v0_10_2::{AddressFilter, BlockId, BlockTag, EmittedEvent, FinalityStatus, TxnFinalityStatus};
use starknet_types_core::felt::Felt;
use std::collections::HashSet;

use super::{ADDRESS_FILTER_LIMIT, BLOCK_PAST_LIMIT};

type ReorgStream = mc_db::subscription::SubscribeReorgs<mc_db::rocksdb::RocksDBStorage>;
type EventEmissionKey = (Felt, u64, TxnFinalityStatus);

enum BackfillResult {
    Restart,
    Complete,
    Closed,
}

enum LiveResult {
    Restart,
    Closed,
}

#[derive(Debug, Clone, Default)]
struct AddressSubscriptionFilter {
    db_from_address: Option<Felt>,
    allowed_addresses: Option<HashSet<Felt>>,
}

struct EventSubscriptionState<'a> {
    sink: &'a jsonrpsee::core::server::SubscriptionSink,
    ctx: &'a crate::WsSubscriptionGuard,
    address_filter: &'a AddressSubscriptionFilter,
    keys: &'a Option<Vec<Vec<Felt>>>,
    reorgs: &'a mut ReorgStream,
    next_block_n: &'a mut u64,
    requested_finality: &'a FinalityStatus,
}

impl AddressSubscriptionFilter {
    fn new(address_filter: Option<&AddressFilter>) -> Self {
        let allowed_addresses = address_filter.and_then(AddressFilter::to_set);
        let db_from_address = allowed_addresses.as_ref().and_then(|addresses| {
            if addresses.len() == 1 {
                addresses.iter().next().copied()
            } else {
                None
            }
        });

        Self { db_from_address, allowed_addresses }
    }

    fn db_from_address(&self) -> Option<Felt> {
        self.db_from_address
    }

    fn matches(&self, event_info: &EventWithInfo) -> bool {
        self.allowed_addresses
            .as_ref()
            .map(|addresses| addresses.contains(&event_info.event.from_address))
            .unwrap_or(true)
    }
}

/// Subscribes to events matching address, key, cursor, and finality filters.
pub async fn subscribe_events(
    starknet: &crate::Starknet,
    subscription_sink: jsonrpsee::PendingSubscriptionSink,
    from_address: Option<AddressFilter>,
    keys: Option<Vec<Vec<Felt>>>,
    block_id: Option<BlockId>,
    finality_status: Option<FinalityStatus>,
) -> Result<(), StarknetWsApiError> {
    if let Err(err) = validate_address_filter(&from_address) {
        subscription_sink.reject(err).await;
        return Ok(());
    }

    if let Err(err) = validate_keys(&keys) {
        subscription_sink.reject(err).await;
        return Ok(());
    }

    let requested_finality = finality_status.unwrap_or_default();
    let address_filter = AddressSubscriptionFilter::new(from_address.as_ref());
    let mut reorgs = starknet.backend.subscribe_reorgs();
    let mut next_block_n = match initial_event_block_n(starknet, block_id) {
        Ok(block_n) => block_n,
        Err(err) => {
            subscription_sink.reject(err).await;
            return Ok(());
        }
    };

    let sink = subscription_sink.accept().await.or_internal_server_error("Failed to establish websocket connection")?;
    let ctx = starknet.ws_handles.subscription_register(sink.subscription_id(), crate::metrics::SUBSCRIBE_EVENTS).await;

    loop {
        let mut state = EventSubscriptionState {
            sink: &sink,
            ctx: &ctx,
            address_filter: &address_filter,
            keys: &keys,
            reorgs: &mut reorgs,
            next_block_n: &mut next_block_n,
            requested_finality: &requested_finality,
        };

        match backfill_events(starknet, &mut state).await? {
            BackfillResult::Restart => continue,
            BackfillResult::Complete => {}
            BackfillResult::Closed => return Ok(()),
        }

        match stream_live_events(starknet, &mut state).await? {
            LiveResult::Restart => continue,
            LiveResult::Closed => return Ok(()),
        }
    }
}

/// Chooses the first block to scan. Without an explicit block id, Starknet OpenRPC defaults to
/// latest, so an already-confirmed latest block is replayed before live events are streamed.
fn initial_event_block_n(starknet: &crate::Starknet, block_id: Option<BlockId>) -> Result<u64, StarknetWsApiError> {
    let Some(block_id) = block_id else {
        return Ok(starknet.backend.latest_confirmed_block_n().unwrap_or(0));
    };

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

    Ok(block_n)
}

/// Replays confirmed events from the cursor until the current latest block or a reorg boundary.
async fn backfill_events(
    starknet: &crate::Starknet,
    state: &mut EventSubscriptionState<'_>,
) -> Result<BackfillResult, StarknetWsApiError> {
    let backfill_to_block_n = starknet.backend.view_on_latest().latest_block_n();
    let mut emitted = HashSet::new();

    while backfill_to_block_n.is_some_and(|end_block_n| *state.next_block_n <= end_block_n) {
        if state.sink.is_closed() {
            return Ok(BackfillResult::Closed);
        }
        if state.ctx.is_cancelled() {
            return Err(crate::errors::StarknetWsApiError::SubscriptionClosed);
        }

        match state.reorgs.try_recv() {
            Ok(reorg) => {
                super::send_reorg_notification(state.sink, &reorg).await?;
                *state.next_block_n = reorg.first_reverted_block_n;
                return Ok(BackfillResult::Restart);
            }
            Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => {
                crate::metrics::record_lagged_reorg(crate::metrics::SUBSCRIBE_EVENTS);
                return Err(super::missed_reorg_notifications_error());
            }
            Err(tokio::sync::broadcast::error::TryRecvError::Closed) => {
                return Err(crate::errors::StarknetWsApiError::Internal);
            }
            Err(tokio::sync::broadcast::error::TryRecvError::Empty) => {}
        }

        if state.ctx.is_cancelled() {
            return Err(crate::errors::StarknetWsApiError::SubscriptionClosed);
        }
        let block_number = *state.next_block_n;
        if let Some(reorg) = send_block_events(starknet, state, block_number, &mut emitted).await? {
            super::send_reorg_notification(state.sink, &reorg).await?;
            *state.next_block_n = reorg.first_reverted_block_n;
            return Ok(BackfillResult::Restart);
        }
        *state.next_block_n = (*state.next_block_n).saturating_add(1);
    }

    Ok(BackfillResult::Complete)
}

/// Streams preconfirmed changes and confirmed heads, restarting from the reorg point when needed.
async fn stream_live_events(
    starknet: &crate::Starknet,
    state: &mut EventSubscriptionState<'_>,
) -> Result<LiveResult, StarknetWsApiError> {
    let mut heads = starknet.backend.subscribe_new_heads(SubscribeNewBlocksTag::Preconfirmed);
    heads.set_start_from(*state.next_block_n);
    let mut current_preconfirmed = starknet
        .backend
        .block_view_on_preconfirmed_or_fake()
        .or_internal_server_error("SubscribeEvents failed to create preconfirmed block view")?;
    let mut emitted = HashSet::new();

    loop {
        let block_view = tokio::select! {
            block_view = heads.next_block_view() => block_view,
            reorg = state.reorgs.recv() => {
                match reorg {
                    Ok(reorg) => {
                        super::send_reorg_notification(state.sink, &reorg).await?;
                        *state.next_block_n = reorg.first_reverted_block_n;
                        return Ok(LiveResult::Restart);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        crate::metrics::record_lagged_reorg(crate::metrics::SUBSCRIBE_EVENTS);
                        return Err(super::missed_reorg_notifications_error());
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        return Err(crate::errors::StarknetWsApiError::Internal);
                    }
                }
            },
            _ = current_preconfirmed.wait_until_outdated() => {
                current_preconfirmed.refresh();
                if let Some(reorg) = send_block_events(
                    starknet,
                    state,
                    current_preconfirmed.block_number(),
                    &mut emitted,
                )
                .await?
                {
                    super::send_reorg_notification(state.sink, &reorg).await?;
                    *state.next_block_n = reorg.first_reverted_block_n;
                    return Ok(LiveResult::Restart);
                }
                continue;
            },
            _ = state.sink.closed() => return Ok(LiveResult::Closed),
            _ = state.ctx.cancelled() => return Err(crate::errors::StarknetWsApiError::SubscriptionClosed),
        };

        let block_number = block_view.block_number();
        if block_view.is_confirmed() {
            match crate::resolve_live_confirmed_head(
                &starknet.backend,
                &mut *state.reorgs,
                block_number,
                super::missed_reorg_notifications_error,
            )? {
                crate::LiveConfirmedHeadResolution::Block(_) => {}
                crate::LiveConfirmedHeadResolution::Reorg(reorg) => {
                    super::send_reorg_notification(state.sink, &reorg).await?;
                    *state.next_block_n = reorg.first_reverted_block_n;
                    return Ok(LiveResult::Restart);
                }
                crate::LiveConfirmedHeadResolution::RetryBackfill => {
                    *state.next_block_n = block_number;
                    return Ok(LiveResult::Restart);
                }
            }
        }
        if let Some(reorg) = send_block_events(starknet, state, block_number, &mut emitted).await? {
            super::send_reorg_notification(state.sink, &reorg).await?;
            *state.next_block_n = reorg.first_reverted_block_n;
            return Ok(LiveResult::Restart);
        }

        if let Some(mut preconfirmed) = block_view.into_preconfirmed() {
            preconfirmed.refresh();
            current_preconfirmed = preconfirmed;
        } else {
            current_preconfirmed = starknet
                .backend
                .block_view_on_preconfirmed_or_fake()
                .or_internal_server_error("SubscribeEvents failed to refresh preconfirmed block view")?;
        }
    }
}

fn validate_address_filter(from_address: &Option<AddressFilter>) -> Result<(), StarknetWsApiError> {
    if matches!(from_address, Some(AddressFilter::Multiple(addresses)) if addresses.len() as u64 > ADDRESS_FILTER_LIMIT)
    {
        return Err(StarknetWsApiError::TooManyAddressesInFilter);
    }

    Ok(())
}

fn validate_keys(keys: &Option<Vec<Vec<Felt>>>) -> Result<(), StarknetWsApiError> {
    let total_keys = keys.as_ref().map(|patterns| patterns.iter().map(Vec::len).sum::<usize>()).unwrap_or(0);
    if total_keys > MAX_EVENTS_KEYS {
        return Err(StarknetWsApiError::TooManyKeysInFilter);
    }

    Ok(())
}

/// Emits matching events for one block using a latest DB view.
///
/// The latest view gives one consistent event scan, while reorg checks before and during emission
/// stop the scan before stale events are sent. Finality is derived from the event's preconfirmed
/// flag plus the latest L1-confirmed cursor, then filtered against the subscription finality. The
/// dedup set is scoped to the current backfill/live phase and prevents duplicate emissions when a
/// block is refreshed or revisited before the next restart.
async fn send_block_events(
    starknet: &crate::Starknet,
    state: &mut EventSubscriptionState<'_>,
    block_number: u64,
    emitted: &mut HashSet<EventEmissionKey>,
) -> Result<Option<mc_db::ReorgNotification>, StarknetWsApiError> {
    let view = starknet.backend.view_on_latest();
    let latest_l1_confirmed_block_n = view.latest_l1_confirmed_block_n();

    let events = view
        .get_events(EventFilter {
            start_block: block_number,
            start_event_index: 0,
            end_block: block_number,
            from_address: state.address_filter.db_from_address(),
            keys_pattern: state.keys.clone(),
            max_events: usize::MAX,
        })
        .context("Error getting filtered events")
        .or_internal_server_error("Failed to retrieve filtered events")?;

    if let Some(reorg) = take_pending_reorg(state.reorgs)? {
        return Ok(Some(reorg));
    }

    for event in events {
        if let Some(reorg) = take_pending_reorg(state.reorgs)? {
            return Ok(Some(reorg));
        }
        if !state.address_filter.matches(&event) {
            continue;
        }

        let finality_status = event_finality_status(&event, latest_l1_confirmed_block_n);
        if !subscription_allows_finality(state.requested_finality, finality_status) {
            continue;
        }
        if !emitted.insert((event.transaction_hash, event.event_index_in_block, finality_status)) {
            continue;
        }

        send_event(event, finality_status, state.sink).await?;
    }

    Ok(None)
}

/// Drains one pending reorg notification, converting lag into a terminal subscription error.
fn take_pending_reorg(reorgs: &mut ReorgStream) -> Result<Option<mc_db::ReorgNotification>, StarknetWsApiError> {
    crate::try_recv_live_reorg(reorgs, super::missed_reorg_notifications_error)
}

/// Applies Starknet's event finality filter; PRE_CONFIRMED subscribers also receive later updates.
fn subscription_allows_finality(requested_finality: &FinalityStatus, event_finality: TxnFinalityStatus) -> bool {
    match requested_finality {
        FinalityStatus::PreConfirmed => true,
        FinalityStatus::AcceptedOnL2 => !matches!(event_finality, TxnFinalityStatus::PreConfirmed),
    }
}

/// Converts event storage metadata into the websocket finality status exposed to subscribers.
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
    let item = mp_rpc::v0_10_2::EmittedEventWithFinality { emitted_event, finality_status };
    super::send_starknet_subscription(sink, super::EVENTS_NOTIFICATION_METHOD, &item).await
}

#[cfg(test)]
mod test {
    use super::*;

    use crate::{
        constants::MAX_EVENTS_KEYS,
        test_utils::rpc_test_setup,
        versions::user::v0_10_2::{StarknetWsRpcApiV0_10_2Client, StarknetWsRpcApiV0_10_2Server},
        Starknet,
    };
    use assert_matches::assert_matches;
    use jsonrpsee::{
        core::{client::SubscriptionClientT, params::ObjectParams},
        ws_client::WsClientBuilder,
    };
    use mc_db::preconfirmed::{PreconfirmedBlock, PreconfirmedExecutedTransaction};
    use mp_block::{header::PreconfirmedHeader, FullBlockWithoutCommitments, TransactionWithReceipt};
    use mp_chain_config::StarknetVersion;
    use mp_receipt::{
        ExecutionResources, ExecutionResult, FeePayment, InvokeTransactionReceipt, PriceUnit, TransactionReceipt,
    };
    use mp_rpc::v0_10_2::{AddressFilter, TxnFinalityStatus};
    use mp_transactions::{InvokeTransaction, InvokeTransactionV0, Transaction as MpTransaction};
    use serde_json::Value;
    use std::time::Duration;

    const SERVER_ADDR: &str = "127.0.0.1:0";

    fn transaction_with_event(
        sender_address: Felt,
        transaction_hash: Felt,
        event_from_address: Felt,
    ) -> TransactionWithReceipt {
        TransactionWithReceipt {
            transaction: MpTransaction::Invoke(InvokeTransaction::V0(InvokeTransactionV0 {
                contract_address: sender_address,
                ..Default::default()
            })),
            receipt: TransactionReceipt::Invoke(InvokeTransactionReceipt {
                transaction_hash,
                events: vec![mp_receipt::Event { from_address: event_from_address, keys: vec![], data: vec![] }],
                actual_fee: FeePayment { amount: Felt::from(0u8), unit: PriceUnit::Wei },
                messages_sent: vec![],
                execution_resources: ExecutionResources::default(),
                execution_result: ExecutionResult::Succeeded,
            }),
        }
    }

    fn add_confirmed_event_block(
        backend: &std::sync::Arc<mc_db::MadaraBackend>,
        block_number: u64,
        sender_address: Felt,
        event_from_address: Felt,
        transaction_hash: Felt,
    ) {
        let tx = transaction_with_event(sender_address, transaction_hash, event_from_address);
        let events = tx
            .receipt
            .events()
            .iter()
            .cloned()
            .map(|event| mp_receipt::EventWithTransactionHash { transaction_hash, event })
            .collect::<Vec<_>>();

        backend
            .write_access()
            .add_full_block_with_classes(
                &FullBlockWithoutCommitments {
                    header: PreconfirmedHeader { block_number, ..Default::default() },
                    state_diff: mp_state_update::StateDiff::default(),
                    transactions: vec![tx],
                    events,
                },
                &[],
                false,
            )
            .expect("Storing block");
    }

    async fn start_server(starknet: Starknet) -> (jsonrpsee::server::ServerHandle, String) {
        let server = jsonrpsee::server::Server::builder()
            .set_id_provider(crate::StarknetSubscriptionIdProvider::default())
            .build(SERVER_ADDR)
            .await
            .expect("Starting server");
        let server_url = format!("ws://{}", server.local_addr().expect("Retrieving server local address"));
        let handle = server.start(StarknetWsRpcApiV0_10_2Server::into_rpc(starknet));
        (handle, server_url)
    }

    async fn raw_subscribe_events(
        client: &jsonrpsee::ws_client::WsClient,
        block_id: Option<BlockId>,
    ) -> jsonrpsee::core::client::Subscription<Value> {
        let mut params = ObjectParams::new();
        if let Some(block_id) = block_id {
            params.insert("block_id", block_id).expect("Building subscribeEvents params");
        }
        SubscriptionClientT::subscribe(
            client,
            "starknet_V0_10_2_subscribeEvents",
            params,
            "starknet_V0_10_2_unsubscribe",
        )
        .await
        .expect("starknet_V0_10_2_subscribeEvents")
    }

    #[tokio::test]
    async fn subscribe_events_multiple_addresses_filter_emits_matching_event_v0_10_2() {
        let (backend, starknet) = rpc_test_setup();
        let (_handle, server_url) = start_server(starknet).await;
        let client = WsClientBuilder::default().build(&server_url).await.expect("Failed to start ws client");

        let event_from_address = Felt::from_hex_unchecked("0x1234");
        let second_address = Felt::from_hex_unchecked("0x5678");
        let mut sub = client
            .subscribe_events(Some(AddressFilter::Multiple(vec![event_from_address, second_address])), None, None, None)
            .await
            .expect("Failed subscription");

        let tx_hash = Felt::from_hex_unchecked("0x4242");
        add_confirmed_event_block(&backend, 0, event_from_address, event_from_address, tx_hash);

        let item = tokio::time::timeout(Duration::from_secs(5), sub.next())
            .await
            .expect("Timed out waiting for event")
            .expect("Subscription closed unexpectedly")
            .expect("Failed to retrieve event");

        assert_eq!(item.finality_status, TxnFinalityStatus::L2);
        assert_eq!(item.emitted_event.event.from_address, event_from_address);
    }

    #[tokio::test]
    async fn subscribe_events_empty_address_array_matches_all_v0_10_2() {
        let (backend, starknet) = rpc_test_setup();
        let (_handle, server_url) = start_server(starknet).await;
        let client = WsClientBuilder::default().build(&server_url).await.expect("Failed to start ws client");

        let event_from_address = Felt::from_hex_unchecked("0x1234");
        let mut sub = client
            .subscribe_events(Some(AddressFilter::Multiple(vec![])), None, None, None)
            .await
            .expect("Failed subscription");

        let tx_hash = Felt::from_hex_unchecked("0x4343");
        add_confirmed_event_block(&backend, 0, event_from_address, event_from_address, tx_hash);

        let item = tokio::time::timeout(Duration::from_secs(5), sub.next())
            .await
            .expect("Timed out waiting for event")
            .expect("Subscription closed unexpectedly")
            .expect("Failed to retrieve event");

        assert_eq!(item.emitted_event.event.from_address, event_from_address);
    }

    #[tokio::test]
    async fn subscribe_events_default_starts_from_latest_v0_10_2() {
        let (backend, starknet) = rpc_test_setup();
        let event_from_address = Felt::from_hex_unchecked("0x1234");
        let latest_hash = Felt::from_hex_unchecked("0x4848");
        add_confirmed_event_block(&backend, 0, event_from_address, event_from_address, latest_hash);

        let (_handle, server_url) = start_server(starknet).await;
        let client = WsClientBuilder::default().build(&server_url).await.expect("Failed to start ws client");
        let mut sub = client.subscribe_events(None, None, None, None).await.expect("Failed subscription");

        let item = tokio::time::timeout(Duration::from_secs(5), sub.next())
            .await
            .expect("Timed out waiting for event")
            .expect("Subscription closed unexpectedly")
            .expect("Failed to retrieve event");

        assert_eq!(item.finality_status, TxnFinalityStatus::L2);
        assert_eq!(item.emitted_event.transaction_hash, latest_hash);
    }

    #[tokio::test]
    async fn subscribe_events_rejects_too_many_addresses_v0_10_2() {
        let (_backend, starknet) = rpc_test_setup();
        let (_handle, server_url) = start_server(starknet).await;
        let client = WsClientBuilder::default().build(&server_url).await.expect("Failed to start ws client");

        let size = super::ADDRESS_FILTER_LIMIT as usize + 1;
        let err = client
            .subscribe_events(Some(AddressFilter::Multiple(vec![Felt::ZERO; size])), None, None, None)
            .await
            .expect_err("Subscription should fail");

        assert_matches!(
            err,
            jsonrpsee::core::client::error::Error::Call(err) => {
                assert_eq!(err, crate::errors::StarknetWsApiError::TooManyAddressesInFilter.into());
            }
        );
    }

    #[tokio::test]
    async fn subscribe_events_preconfirmed_finality_emits_preconfirmed_events_v0_10_2() {
        let (backend, starknet) = rpc_test_setup();
        let (_handle, server_url) = start_server(starknet).await;
        let client = WsClientBuilder::default().build(&server_url).await.expect("Failed to start ws client");

        let mut sub = client
            .subscribe_events(None, None, None, Some(FinalityStatus::PreConfirmed))
            .await
            .expect("Failed subscription");

        let event_from_address = Felt::from_hex_unchecked("0x2345");
        let transaction_hash = Felt::from_hex_unchecked("0x4545");
        backend
            .write_access()
            .new_preconfirmed(PreconfirmedBlock::new_with_content(
                PreconfirmedHeader {
                    block_number: 0,
                    protocol_version: StarknetVersion::V0_13_2,
                    ..Default::default()
                },
                vec![PreconfirmedExecutedTransaction {
                    transaction: transaction_with_event(event_from_address, transaction_hash, event_from_address),
                    state_diff: Default::default(),
                    declared_class: None,
                    arrived_at: Default::default(),
                    paid_fee_on_l1: None,
                }],
                vec![],
            ))
            .expect("Failed to store preconfirmed block");

        let item = tokio::time::timeout(Duration::from_secs(5), sub.next())
            .await
            .expect("Timed out waiting for event")
            .expect("Subscription closed unexpectedly")
            .expect("Failed to retrieve event");

        assert_eq!(item.finality_status, TxnFinalityStatus::PreConfirmed);
        assert!(item.emitted_event.block_number.is_none());
        assert_eq!(item.emitted_event.event.from_address, event_from_address);
    }

    #[tokio::test]
    async fn subscribe_events_preconfirmed_append_emits_only_new_events_v0_10_2() {
        let (backend, starknet) = rpc_test_setup();
        backend
            .write_access()
            .new_preconfirmed(PreconfirmedBlock::new(PreconfirmedHeader {
                block_number: 0,
                protocol_version: StarknetVersion::V0_13_2,
                ..Default::default()
            }))
            .expect("Failed to create empty preconfirmed block");

        let (_handle, server_url) = start_server(starknet).await;
        let client = WsClientBuilder::default().build(&server_url).await.expect("Failed to start ws client");
        let mut sub = client
            .subscribe_events(None, None, None, Some(FinalityStatus::PreConfirmed))
            .await
            .expect("Failed subscription");

        let event_from_address = Felt::from_hex_unchecked("0x2345");
        let first_hash = Felt::from_hex_unchecked("0x4646");
        let second_hash = Felt::from_hex_unchecked("0x4747");
        for transaction_hash in [first_hash, second_hash] {
            let executed = vec![PreconfirmedExecutedTransaction {
                transaction: transaction_with_event(event_from_address, transaction_hash, event_from_address),
                state_diff: Default::default(),
                declared_class: None,
                arrived_at: Default::default(),
                paid_fee_on_l1: None,
            }];
            backend
                .write_access()
                .append_to_preconfirmed(0, &executed, std::iter::empty())
                .expect("Failed to append preconfirmed transaction");

            let item = tokio::time::timeout(Duration::from_secs(5), sub.next())
                .await
                .expect("Timed out waiting for appended preconfirmed event")
                .expect("Subscription closed unexpectedly")
                .expect("Failed to retrieve event");

            assert_eq!(item.finality_status, TxnFinalityStatus::PreConfirmed);
            assert_eq!(item.emitted_event.transaction_hash, transaction_hash);
        }
    }

    #[tokio::test]
    async fn subscribe_events_preconfirmed_filter_emits_confirmed_update_v0_10_2() {
        let (backend, starknet) = rpc_test_setup();
        let (_handle, server_url) = start_server(starknet).await;
        let client = WsClientBuilder::default().build(&server_url).await.expect("Failed to start ws client");

        let mut sub = client
            .subscribe_events(None, None, None, Some(FinalityStatus::PreConfirmed))
            .await
            .expect("Failed subscription");

        let event_from_address = Felt::from_hex_unchecked("0x2345");
        let transaction_hash = Felt::from_hex_unchecked("0x5050");
        let tx = transaction_with_event(event_from_address, transaction_hash, event_from_address);
        backend
            .write_access()
            .new_preconfirmed(PreconfirmedBlock::new_with_content(
                PreconfirmedHeader {
                    block_number: 0,
                    protocol_version: StarknetVersion::V0_13_2,
                    ..Default::default()
                },
                vec![PreconfirmedExecutedTransaction {
                    transaction: tx.clone(),
                    state_diff: Default::default(),
                    declared_class: None,
                    arrived_at: Default::default(),
                    paid_fee_on_l1: None,
                }],
                vec![],
            ))
            .expect("Failed to store preconfirmed block");

        let preconfirmed = tokio::time::timeout(Duration::from_secs(5), sub.next())
            .await
            .expect("Timed out waiting for preconfirmed event")
            .expect("Subscription closed unexpectedly")
            .expect("Failed to retrieve preconfirmed event");
        assert_eq!(preconfirmed.finality_status, TxnFinalityStatus::PreConfirmed);
        assert_eq!(preconfirmed.emitted_event.transaction_hash, transaction_hash);

        backend
            .write_access()
            .add_full_block_with_classes(
                &FullBlockWithoutCommitments {
                    header: PreconfirmedHeader {
                        block_number: 0,
                        protocol_version: StarknetVersion::V0_13_2,
                        ..Default::default()
                    },
                    state_diff: Default::default(),
                    transactions: vec![tx],
                    events: vec![mp_receipt::EventWithTransactionHash {
                        transaction_hash,
                        event: mp_receipt::Event { from_address: event_from_address, keys: vec![], data: vec![] },
                    }],
                },
                &[],
                true,
            )
            .expect("Failed to store confirmed block");

        let confirmed = tokio::time::timeout(Duration::from_secs(5), sub.next())
            .await
            .expect("Timed out waiting for confirmed event update")
            .expect("Subscription closed unexpectedly")
            .expect("Failed to retrieve confirmed event update");

        assert_eq!(confirmed.finality_status, TxnFinalityStatus::L2);
        assert_eq!(confirmed.emitted_event.transaction_hash, transaction_hash);
    }

    #[tokio::test]
    async fn subscribe_events_rejects_too_many_keys_v0_10_2() {
        let (_backend, starknet) = rpc_test_setup();
        let (_handle, server_url) = start_server(starknet).await;
        let client = WsClientBuilder::default().build(&server_url).await.expect("Failed to start ws client");

        let err = client
            .subscribe_events(None, Some(vec![vec![Felt::ONE; MAX_EVENTS_KEYS + 1]]), None, None)
            .await
            .expect_err("Subscription should fail");

        assert_matches!(
            err,
            jsonrpsee::core::client::error::Error::Call(err) => {
                assert_eq!(err, crate::errors::StarknetWsApiError::TooManyKeysInFilter.into());
            }
        );
    }

    #[tokio::test]
    async fn subscribe_events_reorg_then_resume_v0_10_2() {
        let (backend, starknet) = rpc_test_setup();
        let (_handle, server_url) = start_server(starknet).await;
        let client = WsClientBuilder::default().build(&server_url).await.expect("Failed to start ws client");

        let block_0_tx_hash = Felt::from_hex_unchecked("0x7000");
        let block_1_tx_hash = Felt::from_hex_unchecked("0x7001");
        let replacement_tx_hash = Felt::from_hex_unchecked("0x7002");
        let event_from_address = Felt::from_hex_unchecked("0x1234");
        add_confirmed_event_block(&backend, 0, event_from_address, event_from_address, block_0_tx_hash);
        add_confirmed_event_block(&backend, 1, event_from_address, event_from_address, block_1_tx_hash);

        let block_0_hash = backend
            .block_view_on_confirmed(0)
            .expect("Retrieving block 0 view")
            .get_block_info()
            .expect("Retrieving block 0 info")
            .block_hash;
        let block_1_hash = backend
            .block_view_on_confirmed(1)
            .expect("Retrieving block 1 view")
            .get_block_info()
            .expect("Retrieving block 1 info")
            .block_hash;

        let mut sub = raw_subscribe_events(&client, None).await;

        // Starknet OpenRPC says an omitted block_id defaults to latest, and both
        // Pathfinder and Juno replay the latest canonical event before live
        // updates. Drain that bootstrap event before triggering the reorg.
        let latest = tokio::time::timeout(Duration::from_secs(5), sub.next())
            .await
            .expect("Timed out waiting for latest event")
            .expect("Subscription closed unexpectedly")
            .expect("Failed to retrieve latest event");
        let latest: mp_rpc::v0_10_2::EmittedEventWithFinality =
            serde_json::from_value(latest).expect("Failed to deserialize latest event item");

        assert_eq!(latest.finality_status, TxnFinalityStatus::L2);
        assert_eq!(latest.emitted_event.transaction_hash, block_1_tx_hash);

        backend.revert_to(&block_0_hash).expect("Revert should succeed");

        let reorg = tokio::time::timeout(Duration::from_secs(5), sub.next())
            .await
            .expect("Timed out waiting for reorg notification")
            .expect("Subscription closed unexpectedly")
            .expect("Failed to retrieve reorg notification");

        assert_eq!(
            reorg,
            serde_json::to_value(mp_rpc::v0_10_2::ReorgData {
                starting_block_hash: block_1_hash,
                starting_block_number: 1,
                ending_block_hash: block_1_hash,
                ending_block_number: 1,
            })
            .expect("Failed to serialize expected reorg notification")
        );

        add_confirmed_event_block(&backend, 1, event_from_address, event_from_address, replacement_tx_hash);

        let next = tokio::time::timeout(Duration::from_secs(5), sub.next())
            .await
            .expect("Timed out waiting for replacement event")
            .expect("Subscription closed unexpectedly")
            .expect("Failed to retrieve replacement event");
        let item: mp_rpc::v0_10_2::EmittedEventWithFinality =
            serde_json::from_value(next).expect("Failed to deserialize event item");

        assert_eq!(item.finality_status, TxnFinalityStatus::L2);
        assert_eq!(item.emitted_event.event.from_address, event_from_address);
    }

    #[tokio::test]
    async fn subscribe_events_reorg_during_backfill_v0_10_2() {
        const BACKFILL_BLOCKS: u64 = 256;
        let (backend, starknet) = rpc_test_setup();
        let (_handle, server_url) = start_server(starknet).await;
        let client = WsClientBuilder::default().build(&server_url).await.expect("Failed to start ws client");

        let event_from_address = Felt::from_hex_unchecked("0x1234");
        let replacement_tx_hash = Felt::from_hex_unchecked("0x9901");
        for n in 0..BACKFILL_BLOCKS {
            add_confirmed_event_block(&backend, n, event_from_address, event_from_address, Felt::from(0x9000_u64 + n));
        }

        let block_0_hash = backend
            .block_view_on_confirmed(0)
            .expect("Retrieving block 0 view")
            .get_block_info()
            .expect("Retrieving block 0 info")
            .block_hash;
        let block_1_hash = backend
            .block_view_on_confirmed(1)
            .expect("Retrieving block 1 view")
            .get_block_info()
            .expect("Retrieving block 1 info")
            .block_hash;
        let previous_head_hash = backend
            .block_view_on_confirmed(BACKFILL_BLOCKS - 1)
            .expect("Retrieving previous head view")
            .get_block_info()
            .expect("Retrieving previous head info")
            .block_hash;

        let mut sub = raw_subscribe_events(&client, Some(BlockId::Number(0))).await;
        backend.revert_to(&block_0_hash).expect("Revert should succeed");

        let expected_reorg = serde_json::to_value(mp_rpc::v0_10_2::ReorgData {
            starting_block_hash: block_1_hash,
            starting_block_number: 1,
            ending_block_hash: previous_head_hash,
            ending_block_number: BACKFILL_BLOCKS - 1,
        })
        .expect("Failed to serialize expected reorg notification");

        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let next = sub
                    .next()
                    .await
                    .expect("Subscription closed unexpectedly")
                    .expect("Failed to retrieve backfill event");

                if next == expected_reorg {
                    break;
                }
            }
        })
        .await
        .expect("Timed out waiting for reorg notification after event replay");

        add_confirmed_event_block(&backend, 1, event_from_address, event_from_address, replacement_tx_hash);

        let next = tokio::time::timeout(Duration::from_secs(5), sub.next())
            .await
            .expect("Timed out waiting for replacement event")
            .expect("Subscription closed unexpectedly")
            .expect("Failed to retrieve replacement event");
        let item: mp_rpc::v0_10_2::EmittedEventWithFinality =
            serde_json::from_value(next).expect("Failed to deserialize replacement event");

        assert_eq!(item.finality_status, TxnFinalityStatus::L2);
        assert_eq!(item.emitted_event.transaction_hash, replacement_tx_hash);
    }
}
