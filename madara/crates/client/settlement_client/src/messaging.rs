use crate::client::{ClientType, SettlementLayerProvider};
use crate::error::SettlementClientError;
use crate::{RECONNECT_BASE_DELAY, RECONNECT_MAX_DELAY};
use alloy::primitives::{B256, U256};
use futures::StreamExt;
use mc_db::MadaraBackend;
use mp_convert::L1TransactionHash;
use mp_transactions::L1HandlerTransactionWithFee;
use mp_utils::service::ServiceContext;
use starknet_types_core::felt::Felt;
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;

mod find_start_block;

/// Interval for polling the stream and checking finality on queued events.
const STREAM_POLL_INTERVAL: Duration = Duration::from_millis(100);

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

/// Processes events from the queue that have reached the required confirmation depth.
///
/// For each event at the front of the queue, in order:
///
///   1. **Confirmation check**: requires `latest - event_block >= finality_blocks`. If not yet
///      satisfied, leave the event in the queue and stop (events are ordered, so later events
///      are also not yet confirmed).
///
///   2. **Canonical block check** (RPC call — done BEFORE popping the event): query the current
///      canonical block hash at `event.l1_block_number` and compare against `event.l1_block_hash`
///      (captured at observation time). If the RPC call fails transiently, the event stays in the
///      queue and is retried on the next poll. If the hashes differ, the block was reorged out —
///      pop and drop the event WITHOUT writing any nonce metadata and WITHOUT advancing the sync
///      tip (so reconnection can re-scan the reorged region for new canonical events).
///
///   3. **Validity check**: only after the canonical check passes, pop the event, write nonce
///      metadata, and run `check_message_to_l2_validity` (existence check on the L1 contract +
///      cancellation check). If valid, queue for L2 inclusion via `write_pending_message_to_l2`.
///
/// Note: there is still a small unprotected window between queue-write and L2 block production
/// inclusion. Within this window, a deeper-than-`finality_blocks` reorg could invalidate a message
/// that has already been queued. This is accepted as part of the probabilistic safety envelope
/// implied by `finality_blocks` — chosen specifically to make such reorgs negligibly unlikely.
enum FinalizedEventPoll {
    Ready { event: MessageToL2WithMetadata, confirmations: u64 },
    Dropped,
    Waiting,
    RetryCanonicalCheck,
}

/// Inspects the front event without removing it until finality and canonicality are known.
/// Reorged events are discarded, while transient RPC failures leave the event queued for retry.
async fn poll_finalized_event(
    settlement_client: &Arc<dyn SettlementLayerProvider>,
    pending_events: &mut VecDeque<MessageToL2WithMetadata>,
    latest_l1_block: u64,
    finality_blocks: u64,
) -> FinalizedEventPoll {
    let Some(event) = pending_events.front() else {
        return FinalizedEventPoll::Waiting;
    };
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
        Some(hash) if hash == block_hash => {
            FinalizedEventPoll::Ready { event: pending_events.pop_front().expect("front() was Some"), confirmations }
        }
        Some(hash) => {
            let event = pending_events.pop_front().expect("front() was Some");
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
            let event = pending_events.pop_front().expect("front() was Some");
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
) -> Result<L1TransactionHash, SettlementClientError> {
    let nonce = event.message.tx.nonce;
    let l1_tx_hash = L1TransactionHash(event.l1_transaction_hash.to_be_bytes::<32>());
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
    Ok(l1_tx_hash)
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
    let _l1_tx_hash = persist_message_origin(backend, event)?;
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

    loop {
        let (event, confirmations) =
            match poll_finalized_event(settlement_client, pending_events, latest_l1_block, finality_blocks).await {
                FinalizedEventPoll::Ready { event, confirmations } => (event, confirmations),
                FinalizedEventPoll::Dropped => continue,
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
            &event,
            unsafe_skip_l1_message_consumed_check,
            metadata_only,
        )
        .await?;

        // Write the l1 message sync tip
        // This is a marker for l1 sync service to start from later on
        backend
            .write_l1_messaging_sync_tip(Some(event.l1_block_number))
            .map_err(|e| SettlementClientError::DatabaseError(format!("Failed to update sync tip: {}", e)))?;

        notify_consumer.notify_waiters();
    }

    Ok(())
}

#[cfg(test)]
mod tests;
