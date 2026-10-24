use crate::client::{ClientType, SettlementLayerProvider};
use crate::error::SettlementClientError;
use crate::{RECONNECT_BASE_DELAY, RECONNECT_MAX_DELAY};
use alloy::primitives::{B256, U256};
use futures::StreamExt;
use mc_db::MadaraBackend;
use mc_telemetry::{register_counter_metric_instrument, register_gauge_metric_instrument};
use mp_convert::L1TransactionHash;
use mp_transactions::L1HandlerTransactionWithFee;
use mp_utils::service::ServiceContext;
use opentelemetry::global;
use opentelemetry::metrics::{Counter, Gauge};
use starknet_types_core::felt::Felt;
use std::collections::VecDeque;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::Notify;

mod find_start_block;

/// Interval for polling the stream and checking finality on queued events.
const STREAM_POLL_INTERVAL: Duration = Duration::from_millis(100);
#[cfg(not(test))]
const L1_SCAN_POLL_INTERVAL: Duration = Duration::from_secs(12);
#[cfg(test)]
const L1_SCAN_POLL_INTERVAL: Duration = Duration::from_millis(100);
const L1_SCAN_RANGE_SIZE: u64 = 256;
const L1_SCAN_OVERLAP: u64 = 128;
const L1_RECOVERY_FROM_BLOCK_ENV: &str = "MADARA_L1_MESSAGE_RECOVERY_FROM_BLOCK";

struct L1MessageScanMetrics {
    scanned_block: Gauge<u64>,
    lag_blocks: Gauge<u64>,
    last_success_unix: Gauge<u64>,
    failures: Counter<u64>,
    events: Counter<u64>,
}

fn l1_message_scan_metrics() -> &'static L1MessageScanMetrics {
    static METRICS: OnceLock<L1MessageScanMetrics> = OnceLock::new();
    METRICS.get_or_init(|| {
        let meter = global::meter("mc-settlement-client.l1-message-scan");
        L1MessageScanMetrics {
            scanned_block: register_gauge_metric_instrument(
                &meter,
                "l1_message_scanned_block".into(),
                "Last fully scanned Ethereum block".into(),
                "block".into(),
            ),
            lag_blocks: register_gauge_metric_instrument(
                &meter,
                "l1_message_scan_lag_blocks".into(),
                "Confirmed blocks behind the message scanner".into(),
                "block".into(),
            ),
            last_success_unix: register_gauge_metric_instrument(
                &meter,
                "l1_message_scan_last_success_unix".into(),
                "Last successful range scan Unix time".into(),
                "s".into(),
            ),
            failures: register_counter_metric_instrument(
                &meter,
                "l1_message_scan_failures".into(),
                "Failed message range scans".into(),
                "".into(),
            ),
            events: register_counter_metric_instrument(
                &meter,
                "l1_message_scan_events".into(),
                "Message logs returned by range scans, including overlap".into(),
                "".into(),
            ),
        }
    })
}

#[derive(Clone, Debug)]
pub struct MessageToL2WithMetadata {
    pub l1_block_number: u64,
    /// Block hash of the L1 block containing this event at the time it was observed.
    /// Used at processing time to detect reorgs by comparing against the current canonical
    /// hash at `l1_block_number`. If they differ, the block was reorged out and the message
    /// must be dropped.
    pub l1_block_hash: [u8; 32],
    pub l1_transaction_hash: U256,
    pub message: L1HandlerTransactionWithFee,
}

/// Returns true if the message is valid, can be consumed.
///
/// When `unsafe_skip_l1_message_consumed_check` is true, the check against the core contract's
/// `l1ToL2Messages(hash)` mapping is skipped. This means messages that have already been consumed
/// on L1 (refcount == 0 after a state update) will still be considered valid. The local nonce
/// check and cancellation check remain active.
pub async fn check_message_to_l2_validity(
    settlement_client: &Arc<dyn SettlementLayerProvider>,
    backend: &MadaraBackend,
    tx: &L1HandlerTransactionWithFee,
    unsafe_skip_l1_message_consumed_check: bool,
) -> Result<bool, SettlementClientError> {
    // Skip if already processed.
    if backend
        .get_l1_handler_txn_hash_by_nonce(tx.tx.nonce)
        .map_err(|e| SettlementClientError::DatabaseError(format!("Failed to check nonce: {}", e)))?
        .is_some()
    {
        tracing::debug!("Event already processed: {}", tx.tx.nonce);
        return Ok(false);
    }

    // 2 cases for cancellation:
    // * it has been cancelled since (we're reading events from the past) => check that the message still exists.
    // * it is currently being cancelled => we can find this out by checking pending cancellations.

    // Check message hash and cancellation
    let event_hash = settlement_client.calculate_message_hash(tx)?;
    let converted_event_hash = match settlement_client.get_client_type() {
        ClientType::Eth => B256::from_slice(event_hash.as_slice()).to_string(),
        ClientType::Starknet => Felt::from_bytes_be_slice(event_hash.as_slice()).to_hex_string(),
    };
    tracing::debug!("Checking for cancellation, event hash: {:?}", converted_event_hash);

    if unsafe_skip_l1_message_consumed_check {
        tracing::warn!(
            "UNSAFE: Skipping L1 consumed check for message nonce={}, hash={}",
            tx.tx.nonce,
            converted_event_hash
        );
    } else if !settlement_client
        .message_to_l2_is_pending(&event_hash)
        .await
        .map_err(|e| SettlementClientError::InvalidResponse(format!("Failed to check message still exists: {}", e)))?
    {
        tracing::debug!("Message does not exist anymore in core contract.");
        return Ok(false);
    }

    tracing::debug!("Checking for has cancel, event hash: {:?}", converted_event_hash);

    let cancellation_timestamp = settlement_client
        .message_to_l2_has_cancel_request(&event_hash)
        .await
        .map_err(|e| SettlementClientError::InvalidResponse(format!("Failed to check cancellation: {}", e)))?;
    if cancellation_timestamp {
        tracing::debug!("Message is being cancelled");
        return Ok(false);
    }

    Ok(true)
}

pub async fn sync(
    settlement_client: Arc<dyn SettlementLayerProvider>,
    backend: Arc<MadaraBackend>,
    notify_consumer: Arc<Notify>,
    mut ctx: ServiceContext,
    unsafe_skip_l1_message_consumed_check: bool,
    metadata_only: bool,
) -> Result<(), SettlementClientError> {
    // sync inner is cancellation safe.
    ctx.run_until_cancelled(sync_inner(
        settlement_client,
        backend,
        notify_consumer,
        unsafe_skip_l1_message_consumed_check,
        metadata_only,
    ))
    .await
    .transpose()?;
    Ok(())
}

async fn sync_inner(
    settlement_client: Arc<dyn SettlementLayerProvider>,
    backend: Arc<MadaraBackend>,
    notify_consumer: Arc<Notify>,
    unsafe_skip_l1_message_consumed_check: bool,
    metadata_only: bool,
) -> Result<(), SettlementClientError> {
    // Note: It's fine to reprocess events - duplicates are filtered during block production.

    let chain_config = backend.chain_config();
    let replay_max_duration = chain_config.l1_messages_replay_max_duration;
    let finality_blocks = chain_config.l1_messages_finality_blocks;

    let mut reconnect_delay = RECONNECT_BASE_DELAY;

    loop {
        match run_message_sync(
            &settlement_client,
            &backend,
            &notify_consumer,
            replay_max_duration,
            finality_blocks,
            unsafe_skip_l1_message_consumed_check,
            metadata_only,
        )
        .await
        {
            Ok(()) => {
                reconnect_delay = RECONNECT_BASE_DELAY;
            }
            Err(e) => {
                tracing::warn!("L1 message sync failed: {e:#}, reconnecting in {reconnect_delay:?}");
            }
        }

        tokio::time::sleep(reconnect_delay).await;
        reconnect_delay = std::cmp::min(reconnect_delay * 2, RECONNECT_MAX_DELAY);
    }
}

/// Runs a single message sync session. Returns when the stream ends or an error occurs.
async fn run_message_sync(
    settlement_client: &Arc<dyn SettlementLayerProvider>,
    backend: &Arc<MadaraBackend>,
    notify_consumer: &Notify,
    replay_max_duration: Duration,
    finality_blocks: u64,
    unsafe_skip_l1_message_consumed_check: bool,
    metadata_only: bool,
) -> Result<(), SettlementClientError> {
    if matches!(settlement_client.get_client_type(), ClientType::Eth) {
        return run_eth_message_scan(
            settlement_client,
            backend,
            notify_consumer,
            replay_max_duration,
            finality_blocks,
            unsafe_skip_l1_message_consumed_check,
            metadata_only,
        )
        .await;
    }

    let from_l1_block_n = get_start_block(settlement_client, backend, replay_max_duration).await?;

    tracing::info!("⟠ Starting L1→L2 message sync from block #{from_l1_block_n} (finality: {finality_blocks} blocks)");

    let mut stream = settlement_client.messages_to_l2_stream(from_l1_block_n).await?;
    let mut pending_events: VecDeque<MessageToL2WithMetadata> = VecDeque::new();

    loop {
        // Poll stream with timeout. Finality check runs after EVERY iteration (event or timeout).
        // Timeout ensures finality is checked even when L1 is quiet (no new messages arriving).
        let timeout = tokio::time::sleep(STREAM_POLL_INTERVAL);
        tokio::select! {
            biased;

            event = stream.next() => {
                match event {
                    Some(Ok(msg)) => {
                        tracing::debug!(
                            "L1→L2 message received: block={}, nonce={}, tx={:#x}",
                            msg.l1_block_number, msg.message.tx.nonce, msg.l1_transaction_hash
                        );
                        pending_events.push_back(msg);
                    }
                    Some(Err(e)) => {
                        tracing::warn!("L1 event stream error: {e:#}");
                    }
                    None => {
                        // Stream ended - return error to trigger reconnection
                        return Err(SettlementClientError::StreamProcessing(
                            "L1 event stream ended unexpectedly".into(),
                        ));
                    }
                }
            }

            _ = timeout => {}
        }

        // Process finalized events - continue on transient errors
        if let Err(e) = process_finalized_events(
            settlement_client,
            backend,
            notify_consumer,
            &mut pending_events,
            finality_blocks,
            unsafe_skip_l1_message_consumed_check,
            metadata_only,
        )
        .await
        {
            tracing::warn!("Error processing finalized events: {e:#}");
        }
    }
}

fn initial_eth_scan_start(legacy_event_start: u64, recovery_from: Option<u64>) -> u64 {
    legacy_event_start.saturating_sub(L1_SCAN_OVERLAP).min(recovery_from.unwrap_or(u64::MAX))
}

/// Ethereum message ingestion uses explicit block ranges. Only a fully processed range
/// advances the durable cursor; replaying a partially written range is safe by nonce.
async fn run_eth_message_scan(
    settlement_client: &Arc<dyn SettlementLayerProvider>,
    backend: &Arc<MadaraBackend>,
    notify_consumer: &Notify,
    replay_max_duration: Duration,
    finality_blocks: u64,
    unsafe_skip_l1_message_consumed_check: bool,
    metadata_only: bool,
) -> Result<(), SettlementClientError> {
    let mut scanned = backend.get_l1_message_scanned_block().map_err(|error| {
        SettlementClientError::DatabaseError(format!("Failed to get scanned L1 message block: {error}"))
    })?;
    let mut next_block = match scanned {
        Some(block) => block.saturating_add(1),
        None => get_start_block(settlement_client, backend, replay_max_duration).await?,
    };
    // Explicit one-time recovery can begin before the legacy event tip. After a
    // range completes, the durable cursor takes precedence on restarts.
    let recovery_from = match std::env::var(L1_RECOVERY_FROM_BLOCK_ENV) {
        Ok(raw) => Some(raw.parse::<u64>().map_err(|error| {
            SettlementClientError::InvalidResponse(format!(
                "Invalid {L1_RECOVERY_FROM_BLOCK_ENV} block number: {error}"
            ))
        })?),
        Err(std::env::VarError::NotPresent) => None,
        Err(error) => {
            return Err(SettlementClientError::InvalidResponse(format!(
                "Invalid {L1_RECOVERY_FROM_BLOCK_ENV}: {error}"
            )));
        }
    };
    if scanned.is_none() {
        // The legacy tip marks an event, not a complete range. Rewind on
        // migration, or further for a separately reviewed recovery.
        next_block = initial_eth_scan_start(next_block, recovery_from);
        if recovery_from.is_some() {
            tracing::warn!("⟠ Explicit L1 message recovery starts from Ethereum block #{next_block}");
        }
    } else if let Some(recovery_from) = recovery_from {
        tracing::warn!(
            "⟠ {L1_RECOVERY_FROM_BLOCK_ENV} is ignored because a durable scanned cursor already exists; verify the cursor began before block #{recovery_from}"
        );
    }
    tracing::info!(
        "⟠ Starting Ethereum L1 message range scan from block #{next_block} (finality: {finality_blocks} blocks)"
    );
    let metrics = l1_message_scan_metrics();
    if let Some(block) = scanned {
        metrics.scanned_block.record(block, &[]);
    }

    loop {
        let durable_scanned = backend.get_l1_message_scanned_block().map_err(|error| {
            SettlementClientError::DatabaseError(format!("Failed to reread scanned L1 message block: {error}"))
        })?;
        if durable_scanned != scanned {
            // L2 reorg cleanup can rewind this cursor while the scanner is running.
            return Err(SettlementClientError::StreamProcessing(
                "L1 message scan cursor changed externally; restarting from durable value".into(),
            ));
        }
        let head = settlement_client.get_latest_block_number().await?;
        if head < finality_blocks {
            tokio::time::sleep(L1_SCAN_POLL_INTERVAL).await;
            continue;
        }
        let confirmed_head = head.saturating_sub(finality_blocks);
        if next_block > confirmed_head {
            // Recheck the recent overlap even when no new block has arrived. A provider
            // may have returned a successful but incomplete result on an earlier pass.
            if let Some(block) = scanned {
                if let Err(error) = scan_message_range(
                    settlement_client,
                    backend,
                    notify_consumer,
                    block.saturating_sub(L1_SCAN_OVERLAP - 1),
                    block,
                    unsafe_skip_l1_message_consumed_check,
                    metadata_only,
                )
                .await
                {
                    metrics.failures.add(1, &[]);
                    return Err(error);
                }
                metrics.lag_blocks.record(confirmed_head.saturating_sub(block), &[]);
                if let Ok(duration) = SystemTime::now().duration_since(UNIX_EPOCH) {
                    metrics.last_success_unix.record(duration.as_secs(), &[]);
                }
            }
            tokio::time::sleep(L1_SCAN_POLL_INTERVAL).await;
            continue;
        }

        let from = if scanned.is_some() { next_block.saturating_sub(L1_SCAN_OVERLAP) } else { next_block };
        let to = next_block.saturating_add(L1_SCAN_RANGE_SIZE - 1).min(confirmed_head);
        if let Err(error) = scan_message_range(
            settlement_client,
            backend,
            notify_consumer,
            from,
            to,
            unsafe_skip_l1_message_consumed_check,
            metadata_only,
        )
        .await
        {
            metrics.failures.add(1, &[]);
            return Err(error);
        }
        let advanced = backend.compare_and_set_l1_message_scanned_block(scanned, to).map_err(|error| {
            SettlementClientError::DatabaseError(format!("Failed to persist scanned L1 message block {to}: {error}"))
        })?;
        if !advanced {
            return Err(SettlementClientError::StreamProcessing(
                "L1 message scan cursor changed during range; restarting".into(),
            ));
        }
        metrics.scanned_block.record(to, &[]);
        metrics.lag_blocks.record(confirmed_head.saturating_sub(to), &[]);
        if let Ok(duration) = SystemTime::now().duration_since(UNIX_EPOCH) {
            metrics.last_success_unix.record(duration.as_secs(), &[]);
        }
        tracing::info!("⟠ L1 message range scan complete: blocks {from}..={to}, confirmed_head={confirmed_head}");
        scanned = Some(to);
        next_block = to.saturating_add(1);
    }
}

async fn scan_message_range(
    settlement_client: &Arc<dyn SettlementLayerProvider>,
    backend: &MadaraBackend,
    notify_consumer: &Notify,
    from: u64,
    to: u64,
    unsafe_skip_l1_message_consumed_check: bool,
    metadata_only: bool,
) -> Result<(), SettlementClientError> {
    let mut events = settlement_client.messages_to_l2_in_range(from, to).await?;
    events.sort_by_key(|event| (event.l1_block_number, event.message.tx.nonce));
    l1_message_scan_metrics().events.add(events.len() as u64, &[]);
    tracing::debug!("L1 message range {from}..={to} returned {} events", events.len());
    for event in events {
        if event.l1_block_number < from || event.l1_block_number > to {
            return Err(SettlementClientError::InvalidResponse(format!(
                "L1 message log outside requested range {from}..={to}: block {}",
                event.l1_block_number
            )));
        }
        let canonical_hash = settlement_client.get_block_n_hash(event.l1_block_number).await?;
        if canonical_hash != Some(event.l1_block_hash) {
            return Err(SettlementClientError::InvalidResponse(format!(
                "L1 message block {} changed during range scan",
                event.l1_block_number
            )));
        }
        process_canonical_event(
            settlement_client,
            backend,
            &event,
            unsafe_skip_l1_message_consumed_check,
            metadata_only,
        )
        .await?;
        notify_consumer.notify_waiters();
    }
    Ok(())
}

/// Determines the L1 block to start syncing from.
async fn get_start_block(
    settlement_client: &Arc<dyn SettlementLayerProvider>,
    backend: &MadaraBackend,
    replay_max_duration: Duration,
) -> Result<u64, SettlementClientError> {
    // The saved value is the L1 block containing the last processed event, not a fully
    // processed block-range watermark. Resume one block earlier so a crash between two events
    // in the same block cannot skip the later event. This also normalizes provider behavior:
    // Ethereum filters are inclusive, while the Starknet watcher starts after its cursor.
    if let Some(block_n) = backend
        .get_l1_messaging_sync_tip()
        .map_err(|e| SettlementClientError::DatabaseError(format!("Failed to get last synced event block: {}", e)))?
    {
        return Ok(block_n.saturating_sub(1));
    }

    // No saved tip - determine start block based on replay config
    if !replay_max_duration.is_zero() {
        tracing::debug!("Getting latest block_n from settlement layer.");
        let latest_block_n = settlement_client.get_latest_block_number().await?;
        tracing::debug!("Find start, latest {latest_block_n}...");
        find_start_block::find_replay_block_n_start(settlement_client, replay_max_duration, latest_block_n).await
    } else {
        settlement_client.get_latest_block_number().await
    }
}

/// Decision for the borrowed front event; queue ownership stays with the caller.
enum FinalizedEventPoll {
    Ready { confirmations: u64 },
    Dropped,
    Waiting,
    RetryCanonicalCheck,
}

/// Checks finality and canonicality without taking ownership of the event.
/// The caller discards reorged events and retains transient failures for retry.
async fn poll_finalized_event(
    settlement_client: &Arc<dyn SettlementLayerProvider>,
    event: &MessageToL2WithMetadata,
    latest_l1_block: u64,
    finality_blocks: u64,
) -> FinalizedEventPoll {
    let confirmations = latest_l1_block.saturating_sub(event.l1_block_number);
    if confirmations < finality_blocks {
        tracing::debug!(
            "Message at block {} waiting for confirmations: {}/{}",
            event.l1_block_number,
            confirmations,
            finality_blocks
        );
        return FinalizedEventPoll::Waiting;
    }

    let block_number = event.l1_block_number;
    let block_hash = event.l1_block_hash;
    let canonical_hash = match settlement_client.get_block_n_hash(block_number).await {
        Ok(hash) => hash,
        Err(error) => {
            tracing::warn!(
                "Transient failure checking canonical hash at block #{block_number}: {error:#}. \
                 Event stays in queue, will retry on next poll."
            );
            return FinalizedEventPoll::RetryCanonicalCheck;
        }
    };

    match canonical_hash {
        Some(hash) if hash == block_hash => FinalizedEventPoll::Ready { confirmations },
        Some(hash) => {
            tracing::warn!(
                "Dropping reorged L1→L2 message: block={}, nonce={}, observed_hash={:#x}, canonical_hash={:#x}",
                block_number,
                event.message.tx.nonce,
                B256::from(block_hash),
                B256::from(hash),
            );
            FinalizedEventPoll::Dropped
        }
        None => {
            tracing::warn!(
                "Dropping L1→L2 message: block #{} no longer exists on L1 (deep reorg or pruning), nonce={}",
                block_number,
                event.message.tx.nonce,
            );
            FinalizedEventPoll::Dropped
        }
    }
}

/// Stores public L1-origin indexes before the message is considered for L2 inclusion.
/// If execution already consumed the nonce, the reverse status index is backfilled atomically by key.
fn persist_message_origin(
    backend: &MadaraBackend,
    event: &MessageToL2WithMetadata,
) -> Result<(), SettlementClientError> {
    let nonce = event.message.tx.nonce;
    let l1_tx_hash = L1TransactionHash(event.l1_transaction_hash.to_be_bytes::<32>());
    if let Some(existing) = backend.get_l1_txn_hash_by_nonce(nonce).map_err(|error| {
        SettlementClientError::DatabaseError(format!("Failed to read L1 origin for nonce {nonce}: {error}"))
    })? {
        if existing != l1_tx_hash {
            return Err(SettlementClientError::InvalidResponse(format!(
                "Conflicting L1 transaction for message nonce {nonce}; manual reorg reconciliation required"
            )));
        }
    }
    backend.write_l1_txn_hash_by_nonce(nonce, &l1_tx_hash).map_err(|error| {
        SettlementClientError::DatabaseError(format!("Failed to store l1_tx_hash by nonce: {error}"))
    })?;
    backend.insert_message_to_l2_seen_marker(&l1_tx_hash, nonce).map_err(|error| {
        SettlementClientError::DatabaseError(format!("Failed to store l1->l2 sent marker: {error}"))
    })?;

    if let Some(l2_tx_hash) = backend.get_l1_handler_txn_hash_by_nonce(nonce).map_err(|error| {
        SettlementClientError::DatabaseError(format!("Failed to read l1 handler tx hash by nonce: {error}"))
    })? {
        backend.write_message_to_l2_consumed_txn_hash(&l1_tx_hash, nonce, &l2_tx_hash).map_err(|error| {
            SettlementClientError::DatabaseError(format!(
                "Failed to backfill l1->l2 consumed tx hash for (l1_tx_hash, nonce): {error}"
            ))
        })?;
        backend.write_l1_handler_l1_block_by_nonce(nonce, event.l1_block_number).map_err(|error| {
            SettlementClientError::DatabaseError(format!("Failed to store message source block: {error}"))
        })?;
    }
    Ok(())
}

/// Validates one canonical event and optionally queues it for block production.
/// Metadata-only mode retains status indexes while deliberately omitting the pending-message write.
async fn process_canonical_event(
    settlement_client: &Arc<dyn SettlementLayerProvider>,
    backend: &MadaraBackend,
    event: &MessageToL2WithMetadata,
    unsafe_skip_l1_message_consumed_check: bool,
    metadata_only: bool,
) -> Result<(), SettlementClientError> {
    persist_message_origin(backend, event)?;
    let is_valid =
        check_message_to_l2_validity(settlement_client, backend, &event.message, unsafe_skip_l1_message_consumed_check)
            .await
            .map_err(|error| {
                SettlementClientError::InvalidResponse(format!(
                    "Validity check failed for tx {}: {error}",
                    event.l1_transaction_hash
                ))
            })?;

    if is_valid {
        backend.write_l1_handler_l1_block_by_nonce(event.message.tx.nonce, event.l1_block_number).map_err(|error| {
            SettlementClientError::DatabaseError(format!("Failed to store message source block: {error}"))
        })?;
        if metadata_only {
            tracing::debug!(
                "Message metadata stored but NOT queued for block production (unsafe flag): nonce={}",
                event.message.tx.nonce
            );
        } else {
            backend
                .write_pending_message_to_l2(&event.message)
                .map_err(|error| SettlementClientError::DatabaseError(format!("Failed to store message: {error}")))?;
            tracing::debug!("Message stored: nonce={}", event.message.tx.nonce);
        }
    } else {
        tracing::debug!("Message skipped (invalid/cancelled): nonce={}", event.message.tx.nonce);
    }
    Ok(())
}

/// Drains every queued event that has reached finality and passed the canonical-chain check.
/// Processing stops when the front event is still pending or requires a transient retry.
/// Canonical events remain queued until validity checks, metadata writes, and cursor persistence
/// succeed, so a failed event cannot be skipped by a later event advancing the cursor.
/// Reorged events are discarded without advancing the cursor or writing nonce metadata.
/// Reorgs deeper than `finality_blocks` after L2 inclusion remain outside this finality guarantee.
async fn process_finalized_events(
    settlement_client: &Arc<dyn SettlementLayerProvider>,
    backend: &MadaraBackend,
    notify_consumer: &Notify,
    pending_events: &mut VecDeque<MessageToL2WithMetadata>,
    finality_blocks: u64,
    unsafe_skip_l1_message_consumed_check: bool,
    metadata_only: bool,
) -> Result<(), SettlementClientError> {
    if pending_events.is_empty() {
        return Ok(());
    }

    let latest_l1_block = settlement_client.get_latest_block_number().await?;

    while let Some(event) = pending_events.front() {
        let confirmations = match poll_finalized_event(settlement_client, event, latest_l1_block, finality_blocks).await
        {
            FinalizedEventPoll::Ready { confirmations } => confirmations,
            FinalizedEventPoll::Dropped => {
                pending_events.pop_front();
                continue;
            }
            FinalizedEventPoll::Waiting | FinalizedEventPoll::RetryCanonicalCheck => break,
        };

        tracing::info!(
            "Processing L1→L2 message: block={}, nonce={}, confirmations={}",
            event.l1_block_number,
            event.message.tx.nonce,
            confirmations
        );

        process_canonical_event(
            settlement_client,
            backend,
            event,
            unsafe_skip_l1_message_consumed_check,
            metadata_only,
        )
        .await?;

        // Write the l1 message sync tip
        // This is a marker for l1 sync service to start from later on
        backend
            .write_l1_messaging_sync_tip(Some(event.l1_block_number))
            .map_err(|e| SettlementClientError::DatabaseError(format!("Failed to update sync tip: {}", e)))?;

        pending_events.pop_front();
        notify_consumer.notify_waiters();
    }

    Ok(())
}

#[cfg(test)]
mod tests;
