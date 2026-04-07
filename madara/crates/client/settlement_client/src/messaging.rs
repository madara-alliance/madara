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
    // Check if we have a saved sync tip
    if let Some(block_n) = backend
        .get_l1_messaging_sync_tip()
        .map_err(|e| SettlementClientError::DatabaseError(format!("Failed to get last synced event block: {}", e)))?
    {
        return Ok(block_n);
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
///   2. **Canonical block check**: query the current canonical block hash at `event.l1_block_number`
///      and compare against `event.l1_block_hash` (captured at observation time). If they differ,
///      the block was reorged out of the canonical chain — drop the event entirely WITHOUT writing
///      any nonce metadata (otherwise the nonce would be permanently poisoned and a re-emitted
///      message with the same nonce on the new canonical chain would be incorrectly skipped).
///      Still advance the sync tip past the event so we don't keep re-fetching the same range.
///
///   3. **Validity check**: only after the canonical check passes, write nonce metadata and
///      run `check_message_to_l2_validity` (existence check on the L1 contract + cancellation
///      check). If valid, queue for L2 inclusion via `write_pending_message_to_l2`.
///
/// Note: there is still a small unprotected window between queue-write and L2 block production
/// inclusion. Within this window, a deeper-than-`finality_blocks` reorg could invalidate a message
/// that has already been queued. This is accepted as part of the probabilistic safety envelope
/// implied by `finality_blocks` — chosen specifically to make such reorgs negligibly unlikely.
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

    // Process events from the front (oldest first) that have reached the confirmation depth
    while let Some(event) = pending_events.front() {
        let confirmations = latest_l1_block.saturating_sub(event.l1_block_number);

        // Step 1: confirmation depth check
        if confirmations < finality_blocks {
            tracing::debug!(
                "Message at block {} waiting for confirmations: {}/{}",
                event.l1_block_number,
                confirmations,
                finality_blocks
            );
            break; // Events are ordered, remaining events are also not yet confirmed
        }

        // SAFETY: We just confirmed front() returned Some
        let event = pending_events.pop_front().expect("front() was Some");

        // Step 2: canonical block check — guard against reorged-out blocks.
        // We compare the block hash captured at event observation time against the current
        // canonical hash at the same block number. If they differ, the block was reorged out
        // and the event must be dropped without poisoning nonce metadata.
        let canonical_hash = settlement_client.get_block_n_hash(event.l1_block_number).await.map_err(|e| {
            SettlementClientError::InvalidResponse(format!(
                "Failed to fetch canonical hash at block {}: {}",
                event.l1_block_number, e
            ))
        })?;
        match canonical_hash {
            Some(hash) if hash == event.l1_block_hash => {
                // Block is canonical — proceed with processing.
            }
            Some(hash) => {
                tracing::warn!(
                    "Dropping reorged L1→L2 message: block={}, nonce={}, observed_hash={:#x}, canonical_hash={:#x}",
                    event.l1_block_number,
                    event.message.tx.nonce,
                    B256::from(event.l1_block_hash),
                    B256::from(hash),
                );
                // Advance the sync tip so we don't re-fetch this range. Do NOT write any
                // nonce metadata: the nonce may be re-emitted on the new canonical chain,
                // and we must not block its future processing.
                backend
                    .write_l1_messaging_sync_tip(Some(event.l1_block_number))
                    .map_err(|e| SettlementClientError::DatabaseError(format!("Failed to update sync tip: {}", e)))?;
                continue;
            }
            None => {
                tracing::warn!(
                    "Dropping L1→L2 message: block #{} no longer exists on L1 (deep reorg or pruning)",
                    event.l1_block_number,
                );
                backend
                    .write_l1_messaging_sync_tip(Some(event.l1_block_number))
                    .map_err(|e| SettlementClientError::DatabaseError(format!("Failed to update sync tip: {}", e)))?;
                continue;
            }
        }

        tracing::info!(
            "Processing L1→L2 message: block={}, nonce={}, confirmations={}",
            event.l1_block_number,
            event.message.tx.nonce,
            confirmations
        );

        // Step 3: persist origin metadata + validity check + queue for L2 inclusion.
        //
        // Persist minimal origin metadata for `starknet_getMessagesStatus`:
        // - nonce -> l1_tx_hash
        // - l1_tx_hash||nonce -> <empty> (seen marker), later filled with l2_tx_hash once known
        let nonce = event.message.tx.nonce;
        let l1_tx_hash = L1TransactionHash(event.l1_transaction_hash.to_be_bytes::<32>());
        backend
            .write_l1_txn_hash_by_nonce(nonce, &l1_tx_hash)
            .map_err(|e| SettlementClientError::DatabaseError(format!("Failed to store l1_tx_hash by nonce: {}", e)))?;
        backend
            .insert_message_to_l2_seen_marker(&l1_tx_hash, nonce)
            .map_err(|e| SettlementClientError::DatabaseError(format!("Failed to store l1->l2 sent marker: {}", e)))?;

        // Backfill the consumed L2 tx hash in case the L1 handler transaction was already written to the DB
        // before we observed the L1 event for this nonce.
        // Checking nonce -> L2 transaction hash
        if let Some(l2_tx_hash) = backend.get_l1_handler_txn_hash_by_nonce(nonce).map_err(|e| {
            SettlementClientError::DatabaseError(format!("Failed to read l1 handler tx hash by nonce: {}", e))
        })? {
            // Write l1_tx_hash|nonce -> l2_tx_hash
            backend.write_message_to_l2_consumed_txn_hash(&l1_tx_hash, nonce, &l2_tx_hash).map_err(|e| {
                SettlementClientError::DatabaseError(format!(
                    "Failed to backfill l1->l2 consumed tx hash for (l1_tx_hash, nonce): {}",
                    e
                ))
            })?;
            // Write nonce -> l1_block
            backend.write_l1_handler_l1_block_by_nonce(nonce, event.l1_block_number).map_err(|e| {
                SettlementClientError::DatabaseError(format!("Failed to store message source block: {}", e))
            })?;
        }

        // Check if the message is still valid
        let is_valid = check_message_to_l2_validity(
            settlement_client,
            backend,
            &event.message,
            unsafe_skip_l1_message_consumed_check,
        )
        .await
        .map_err(|e| {
            SettlementClientError::InvalidResponse(format!(
                "Validity check failed for tx {}: {}",
                event.l1_transaction_hash, e
            ))
        })?;

        if is_valid {
            // Write nonce -> l1_block
            backend.write_l1_handler_l1_block_by_nonce(event.message.tx.nonce, event.l1_block_number).map_err(|e| {
                SettlementClientError::DatabaseError(format!("Failed to store message source block: {}", e))
            })?;
            if metadata_only {
                tracing::debug!(
                    "Message metadata stored but NOT queued for block production (unsafe flag): nonce={}",
                    event.message.tx.nonce
                );
            } else {
                // Write nonce -> pending_message (to be picked by block production service)
                backend
                    .write_pending_message_to_l2(&event.message)
                    .map_err(|e| SettlementClientError::DatabaseError(format!("Failed to store message: {}", e)))?;
                tracing::debug!("Message stored: nonce={}", event.message.tx.nonce);
            }
        } else {
            tracing::debug!("Message skipped (invalid/cancelled): nonce={}", event.message.tx.nonce);
        }

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
mod messaging_module_tests {
    use super::*;
    use crate::client::MockSettlementLayerProvider;
    use crate::messages_to_l2_consumer::MessagesToL2Consumer;
    use futures::stream;
    use futures::FutureExt;
    use mockall::predicate;
    use mp_chain_config::ChainConfig;
    use mp_transactions::L1HandlerTransaction;
    use rstest::{fixture, rstest};
    use starknet_types_core::felt::Felt;
    use std::time::{Duration, SystemTime};

    // Helper function to create a mock event
    fn create_mock_event(l1_block_number: u64, nonce: u64) -> MessageToL2WithMetadata {
        MessageToL2WithMetadata {
            l1_block_number,
            l1_block_hash: [0u8; 32],
            l1_transaction_hash: U256::from(1),
            message: L1HandlerTransactionWithFee::new(
                L1HandlerTransaction {
                    version: Felt::ZERO,
                    nonce,
                    contract_address: Felt::from(456),
                    entry_point_selector: Felt::from(789),
                    calldata: vec![Felt::from(123), Felt::from(1), Felt::from(2)].into(),
                },
                1000,
            ),
        }
    }

    struct MessagingTestRunner {
        client: MockSettlementLayerProvider,
        db: Arc<MadaraBackend>,
        ctx: ServiceContext,
    }

    #[fixture]
    async fn setup_messaging_tests(#[default(false)] msg_replay_enabled: bool) -> MessagingTestRunner {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .with_test_writer()
            .try_init();
        // Set up chain info
        let mut chain_config = ChainConfig::madara_test();
        if msg_replay_enabled {
            chain_config.l1_messages_replay_max_duration = Duration::from_secs(30);
        } else {
            chain_config.l1_messages_replay_max_duration = Default::default();
        }
        let chain_config = Arc::new(chain_config);

        // Initialize database service
        let db = MadaraBackend::open_for_testing(chain_config.clone());

        // Create a mock client directly
        let mut mock_client = MockSettlementLayerProvider::new();

        // Configure basic mock expectations that all tests will need
        mock_client.expect_get_client_type().returning(|| ClientType::Eth);

        // Create a new service context for testing
        let ctx = ServiceContext::new_for_testing();

        MessagingTestRunner { client: mock_client, db, ctx }
    }

    fn mock_l1_handler_tx(mock: &mut MockSettlementLayerProvider, nonce: u64, is_pending: bool, has_cancel_req: bool) {
        tracing::debug!("{:?}", create_mock_event(0, nonce).message);
        mock.expect_calculate_message_hash()
            .with(predicate::eq(create_mock_event(0, nonce).message))
            .returning(move |_| Ok(vec![nonce as u8; 32]));
        mock.expect_message_to_l2_has_cancel_request()
            .with(predicate::eq(vec![nonce as u8; 32]))
            .returning(move |_| Ok(has_cancel_req));
        mock.expect_message_to_l2_is_pending()
            .with(predicate::eq(vec![nonce as u8; 32]))
            .returning(move |_| Ok(is_pending));
    }

    /// Mocks the canonical block hash check so that any event from `create_mock_event` passes
    /// the reorg check in `process_finalized_events`. The default mock_event uses
    /// `l1_block_hash = [0u8; 32]`, so we return that for every block number.
    fn mock_canonical_block_hash(mock: &mut MockSettlementLayerProvider) {
        mock.expect_get_block_n_hash().returning(|_| Ok(Some([0u8; 32])));
    }

    #[rstest]
    #[tokio::test]
    async fn test_sync_processes_new_messages(
        #[future] setup_messaging_tests: MessagingTestRunner,
    ) -> anyhow::Result<()> {
        let MessagingTestRunner { mut client, db, ctx } = setup_messaging_tests.await;

        // Setup mock event and configure backend
        let mock_event1 = create_mock_event(100, 1);
        let notify = Arc::new(Notify::new());

        // Setup mock for last synced block (avoids calling find_replay_block_n_start)
        db.write_l1_messaging_sync_tip(Some(99))?;

        // Mock get_messaging_stream
        let events = vec![mock_event1.clone()];
        client.expect_messages_to_l2_stream().returning(move |_| Ok(stream::iter(events.clone()).map(Ok).boxed()));

        // Mock get_latest_block_number (needed for finality check)
        // Event is at block 100, latest is 200, so finality check passes (default finality_blocks=10)
        client.expect_get_latest_block_number().returning(|| Ok(200));

        // Mock canonical block hash check (event passes reorg check)
        mock_canonical_block_hash(&mut client);

        // nonce 1, is pending, not being canceled, not consumed in db. => OK
        mock_l1_handler_tx(&mut client, 1, true, false);
        db.write_l1_handler_txn_hash_by_nonce(18, &Felt::ONE).unwrap();

        // Mock get_client_type
        client.expect_get_client_type().returning(|| ClientType::Eth);

        // Wrap the client in Arc
        let client = Arc::new(client) as Arc<dyn SettlementLayerProvider>;

        // Keep a reference to context for cancellation
        let ctx_clone = ctx.clone();
        let db_backend_clone = db.clone();

        // Spawn the sync task in a separate thread
        let sync_handle = tokio::spawn(async move { sync(client, db_backend_clone, notify, ctx, false, false).await });

        // Wait for event to be processed (short wait since stream returns immediately)
        tokio::time::sleep(Duration::from_millis(500)).await;

        // Verify the message was processed
        assert_eq!(db.get_pending_message_to_l2(mock_event1.message.tx.nonce).unwrap().unwrap(), mock_event1.message);
        let l1_tx_hash = L1TransactionHash(mock_event1.l1_transaction_hash.to_be_bytes::<32>());
        assert_eq!(db.get_l1_txn_hash_by_nonce(mock_event1.message.tx.nonce).unwrap(), Some(l1_tx_hash));
        assert_eq!(
            db.get_messages_to_l2_by_l1_tx_hash(&l1_tx_hash).unwrap().unwrap(),
            vec![(mock_event1.message.tx.nonce, None)]
        );

        // Clean up: cancel context and abort task
        ctx_clone.cancel_global();
        sync_handle.abort();

        Ok(())
    }

    #[rstest]
    #[tokio::test]
    /// Ensures `getMessagesStatus` metadata is consistent when the L1 event is observed *after* the L2 execution.
    ///
    /// Desired results:
    /// - The message is not re-queued as pending (since `nonce -> l2_tx_hash` already exists).
    /// - The `(l1_tx_hash||nonce)` secondary index is backfilled with the already-known L2 tx hash.
    async fn test_sync_backfills_consumed_tx_hash_when_already_known(
        #[future] setup_messaging_tests: MessagingTestRunner,
    ) -> anyhow::Result<()> {
        let MessagingTestRunner { mut client, db, ctx } = setup_messaging_tests.await;

        // Setup mock event and configure backend
        let mock_event1 = create_mock_event(100, 1);
        let notify = Arc::new(Notify::new());

        // Setup mock for last synced block (avoids calling find_replay_block_n_start)
        db.write_l1_messaging_sync_tip(Some(99))?;

        // Pretend the L1 handler tx was already executed and stored before we observed the L1 event.
        let consumed_l2_tx_hash = Felt::from_hex_unchecked("0x123");
        db.write_l1_handler_txn_hash_by_nonce(mock_event1.message.tx.nonce, &consumed_l2_tx_hash)?;

        // Mock get_messaging_stream
        let events = vec![mock_event1.clone()];
        client.expect_messages_to_l2_stream().returning(move |_| Ok(stream::iter(events.clone()).map(Ok).boxed()));

        // Mock get_latest_block_number (needed for finality check)
        // Event is at block 100, latest is 200, so finality check passes (default finality_blocks=10)
        client.expect_get_latest_block_number().returning(|| Ok(200));

        // Mock canonical block hash check (event passes reorg check)
        mock_canonical_block_hash(&mut client);

        // Mock get_client_type
        client.expect_get_client_type().returning(|| ClientType::Eth);

        // Wrap the client in Arc
        let client = Arc::new(client) as Arc<dyn SettlementLayerProvider>;

        // Keep a reference to context for cancellation
        let ctx_clone = ctx.clone();
        let db_backend_clone = db.clone();

        // Spawn the sync task in a separate thread
        let sync_handle = tokio::spawn(async move { sync(client, db_backend_clone, notify, ctx, false, false).await });

        // Wait for event to be processed (short wait since stream returns immediately)
        tokio::time::sleep(Duration::from_millis(500)).await;

        // Verify the message was not re-queued as pending (it is already processed).
        assert!(db.get_pending_message_to_l2(mock_event1.message.tx.nonce).unwrap().is_none());
        let l1_tx_hash = mp_convert::L1TransactionHash(mock_event1.l1_transaction_hash.to_be_bytes::<32>());
        assert_eq!(db.get_l1_txn_hash_by_nonce(mock_event1.message.tx.nonce).unwrap(), Some(l1_tx_hash));
        assert_eq!(
            db.get_messages_to_l2_by_l1_tx_hash(&l1_tx_hash).unwrap().unwrap(),
            vec![(mock_event1.message.tx.nonce, Some(consumed_l2_tx_hash))]
        );

        // Clean up: cancel context and abort task
        ctx_clone.cancel_global();
        sync_handle.abort();

        Ok(())
    }

    #[rstest]
    #[tokio::test]
    async fn test_sync_catches_earlier_messages(
        #[future]
        #[with(/* enable replay */ true)]
        setup_messaging_tests: MessagingTestRunner,
    ) -> anyhow::Result<()> {
        let MessagingTestRunner { mut client, db, ctx } = setup_messaging_tests.await;

        // Setup mock event and configure backend
        let mock_event1 = create_mock_event(55, 1);
        let notify = Arc::new(Notify::new());

        let current_timestamp_secs = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("Current time is before UNIX_EPOCH")
            .as_secs();

        for block in 0..100 {
            client
                .expect_get_block_n_timestamp()
                .with(predicate::eq(100 - block))
                .returning(move |_| Ok(current_timestamp_secs - block * 2));
        }
        client.expect_get_latest_block_number().returning(move || Ok(100));

        // Mock canonical block hash check (event passes reorg check)
        mock_canonical_block_hash(&mut client);

        let from_l1_block_n = 84; // it should find this block

        // Mock get_messaging_stream
        let events = vec![mock_event1.clone()];
        client
            .expect_messages_to_l2_stream()
            .with(predicate::eq(from_l1_block_n))
            .returning(move |_| Ok(stream::iter(events.clone()).map(Ok).boxed()));

        // nonce 1, is pending, not being cancelled, not consumed in db. => OK
        mock_l1_handler_tx(&mut client, 1, true, false);

        // Mock get_client_type
        client.expect_get_client_type().returning(|| ClientType::Eth);

        // Wrap the client in Arc
        let client = Arc::new(client) as Arc<dyn SettlementLayerProvider>;

        // Keep a reference to context for cancellation
        let ctx_clone = ctx.clone();
        let db_backend_clone = db.clone();

        // Spawn the sync task in a separate thread
        let sync_handle = tokio::spawn(async move { sync(client, db_backend_clone, notify, ctx, false, false).await });

        // Wait for event to be processed (short wait since stream returns immediately)
        tokio::time::sleep(Duration::from_millis(500)).await;

        // Verify the message was processed
        assert_eq!(db.get_pending_message_to_l2(mock_event1.message.tx.nonce).unwrap().unwrap(), mock_event1.message);
        let l1_tx_hash = L1TransactionHash(mock_event1.l1_transaction_hash.to_be_bytes::<32>());
        assert_eq!(db.get_l1_txn_hash_by_nonce(mock_event1.message.tx.nonce).unwrap(), Some(l1_tx_hash));
        assert_eq!(
            db.get_messages_to_l2_by_l1_tx_hash(&l1_tx_hash).unwrap().unwrap(),
            vec![(mock_event1.message.tx.nonce, None)]
        );

        // Clean up: cancel context and abort task
        ctx_clone.cancel_global();
        sync_handle.abort();

        Ok(())
    }

    #[tokio::test]
    async fn test_finality_blocks_delays_processing() -> anyhow::Result<()> {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .with_test_writer()
            .try_init();

        // Set up chain config with finality requirement of 10 blocks
        let mut chain_config = ChainConfig::madara_test();
        chain_config.l1_messages_finality_blocks = 10;
        let chain_config = Arc::new(chain_config);

        let db = MadaraBackend::open_for_testing(chain_config.clone());

        // Set sync tip to avoid calling find_replay_block_n_start
        db.write_l1_messaging_sync_tip(Some(99))?;

        let mut mock_client = MockSettlementLayerProvider::new();

        // Event at block 100
        let mock_event = create_mock_event(100, 1);

        // Mock get_messaging_stream
        let events = vec![mock_event.clone()];
        mock_client.expect_messages_to_l2_stream().returning(move |_| Ok(stream::iter(events.clone()).map(Ok).boxed()));

        // Latest block is 105 - only 5 blocks after event, less than 10 required
        // Event should NOT be processed yet
        mock_client.expect_get_latest_block_number().returning(|| Ok(105));

        mock_client.expect_get_client_type().returning(|| ClientType::Eth);

        let client = Arc::new(mock_client) as Arc<dyn SettlementLayerProvider>;
        let ctx = ServiceContext::new_for_testing();
        let ctx_clone = ctx.clone();
        let notify = Arc::new(Notify::new());
        let db_clone = db.clone();

        let sync_handle = tokio::spawn(async move { sync(client, db_clone, notify, ctx, false, false).await });

        // Wait for processing attempt (short wait)
        tokio::time::sleep(Duration::from_millis(500)).await;

        // Event should NOT be processed (not finalized yet)
        assert!(
            db.get_pending_message_to_l2(mock_event.message.tx.nonce).unwrap().is_none(),
            "Event should not be processed before finality threshold"
        );

        ctx_clone.cancel_global();
        sync_handle.abort();

        Ok(())
    }

    #[tokio::test]
    async fn test_finality_blocks_processes_after_threshold() -> anyhow::Result<()> {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .with_test_writer()
            .try_init();

        // Set up chain config with finality requirement of 10 blocks
        let mut chain_config = ChainConfig::madara_test();
        chain_config.l1_messages_finality_blocks = 10;
        let chain_config = Arc::new(chain_config);

        let db = MadaraBackend::open_for_testing(chain_config.clone());

        // Set sync tip to avoid calling find_replay_block_n_start
        db.write_l1_messaging_sync_tip(Some(99))?;

        let mut mock_client = MockSettlementLayerProvider::new();

        // Event at block 100
        let mock_event = create_mock_event(100, 1);

        // Mock get_messaging_stream
        let events = vec![mock_event.clone()];
        mock_client.expect_messages_to_l2_stream().returning(move |_| Ok(stream::iter(events.clone()).map(Ok).boxed()));

        // Latest block is 115 - 15 blocks after event, more than 10 required
        // Event SHOULD be processed
        mock_client.expect_get_latest_block_number().returning(|| Ok(115));

        mock_client.expect_get_client_type().returning(|| ClientType::Eth);
        mock_l1_handler_tx(&mut mock_client, 1, true, false);
        mock_canonical_block_hash(&mut mock_client);

        let client = Arc::new(mock_client) as Arc<dyn SettlementLayerProvider>;
        let ctx = ServiceContext::new_for_testing();
        let ctx_clone = ctx.clone();
        let notify = Arc::new(Notify::new());
        let db_clone = db.clone();

        let sync_handle = tokio::spawn(async move { sync(client, db_clone, notify, ctx, false, false).await });

        // Wait for processing (short wait)
        tokio::time::sleep(Duration::from_millis(500)).await;

        // Event SHOULD be processed (finalized)
        assert!(
            db.get_pending_message_to_l2(mock_event.message.tx.nonce).unwrap().is_some(),
            "Event should be processed after finality threshold"
        );

        ctx_clone.cancel_global();
        sync_handle.abort();

        Ok(())
    }

    /// Verifies the canonical block hash check correctly drops events whose source block
    /// no longer exists on the canonical chain (e.g., after a deep reorg or pruning).
    ///
    /// Critical assertion: when an event is dropped because its block doesn't exist, the
    /// nonce metadata must NOT be written. Otherwise the nonce would be permanently poisoned
    /// and a re-emitted message with the same nonce on the new canonical chain would be
    /// incorrectly skipped.
    #[tokio::test]
    async fn test_drops_event_when_block_does_not_exist() -> anyhow::Result<()> {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .with_test_writer()
            .try_init();

        let mut chain_config = ChainConfig::madara_test();
        chain_config.l1_messages_finality_blocks = 10;
        let chain_config = Arc::new(chain_config);
        let db = MadaraBackend::open_for_testing(chain_config.clone());

        // Set sync tip to avoid calling find_replay_block_n_start
        db.write_l1_messaging_sync_tip(Some(99))?;

        let mut mock_client = MockSettlementLayerProvider::new();

        // Event at block 100, latest at 115 → confirmation check passes
        let mock_event = create_mock_event(100, 1);
        let events = vec![mock_event.clone()];
        mock_client.expect_messages_to_l2_stream().returning(move |_| Ok(stream::iter(events.clone()).map(Ok).boxed()));
        mock_client.expect_get_latest_block_number().returning(|| Ok(115));
        mock_client.expect_get_client_type().returning(|| ClientType::Eth);

        // CRITICAL: get_block_n_hash returns None — block at #100 no longer exists on canonical chain.
        mock_client.expect_get_block_n_hash().returning(|_| Ok(None));

        // Note: NO mock for calculate_message_hash / message_to_l2_is_pending /
        // message_to_l2_has_cancel_request — those should NOT be called because the
        // canonical check fails first, short-circuiting validity checks.

        let client = Arc::new(mock_client) as Arc<dyn SettlementLayerProvider>;
        let ctx = ServiceContext::new_for_testing();
        let ctx_clone = ctx.clone();
        let notify = Arc::new(Notify::new());
        let db_clone = db.clone();

        let sync_handle = tokio::spawn(async move { sync(client, db_clone, notify, ctx, false, false).await });

        tokio::time::sleep(Duration::from_millis(500)).await;

        // The event must NOT have been queued for L2 inclusion
        assert!(
            db.get_pending_message_to_l2(mock_event.message.tx.nonce).unwrap().is_none(),
            "Reorged event should not be in pending_message_to_l2"
        );

        // CRITICAL: nonce metadata must NOT have been written (no poisoning)
        assert!(
            db.get_l1_txn_hash_by_nonce(mock_event.message.tx.nonce).unwrap().is_none(),
            "Nonce should NOT be poisoned after dropping a reorged event"
        );

        // Sync tip should still have advanced past the dropped event so we don't re-fetch the same range
        let sync_tip = db.get_l1_messaging_sync_tip().unwrap().unwrap();
        assert!(
            sync_tip >= mock_event.l1_block_number,
            "Sync tip should have advanced past the dropped event's block"
        );

        ctx_clone.cancel_global();
        sync_handle.abort();

        Ok(())
    }

    /// Tests that sync_inner implements exponential backoff when L1 RPC fails.
    /// Verifies: no panic, retries continue, backoff doubles, caps at max delay.
    ///
    /// `start_paused = true` pauses tokio's time at test start, allowing us to
    /// manually advance time with `tokio::time::advance()` for deterministic testing.
    #[tokio::test(start_paused = true)]
    async fn test_backoff_on_rpc_failure() -> anyhow::Result<()> {
        use std::sync::atomic::{AtomicU32, Ordering};

        let chain_config = Arc::new(ChainConfig::madara_test());
        let db = MadaraBackend::open_for_testing(chain_config.clone());
        db.write_l1_messaging_sync_tip(Some(99))?;

        let mut mock_client = MockSettlementLayerProvider::new();
        let attempt_count = Arc::new(AtomicU32::new(0));
        let attempt_count_clone = attempt_count.clone();

        // Simulate RPC failure: stream ends immediately on each attempt
        mock_client.expect_messages_to_l2_stream().returning(move |_| {
            attempt_count_clone.fetch_add(1, Ordering::SeqCst);
            Ok(stream::empty().boxed())
        });
        mock_client.expect_get_client_type().returning(|| ClientType::Eth);

        let client = Arc::new(mock_client);
        let ctx = ServiceContext::new_for_testing();

        let sync_handle =
            tokio::spawn(async move { sync(client, db, Arc::new(Notify::new()), ctx, false, false).await });

        // Verify exponential backoff: 1s -> 2s -> 4s
        tokio::time::advance(Duration::from_millis(50)).await;
        tokio::task::yield_now().await;
        assert_eq!(attempt_count.load(Ordering::SeqCst), 1);

        tokio::time::advance(Duration::from_secs(1)).await;
        tokio::task::yield_now().await;
        assert_eq!(attempt_count.load(Ordering::SeqCst), 2);

        tokio::time::advance(Duration::from_secs(2)).await;
        tokio::task::yield_now().await;
        assert_eq!(attempt_count.load(Ordering::SeqCst), 3);

        // Task should still be running (no panic)
        assert!(!sync_handle.is_finished());

        sync_handle.abort();
        Ok(())
    }

    #[rstest]
    #[tokio::test]
    async fn test_metadata_only_flag_stores_metadata_but_not_pending(
        #[future] setup_messaging_tests: MessagingTestRunner,
    ) -> anyhow::Result<()> {
        let MessagingTestRunner { mut client, db, ctx } = setup_messaging_tests.await;

        let mock_event1 = create_mock_event(100, 1);
        let notify = Arc::new(Notify::new());

        db.write_l1_messaging_sync_tip(Some(99))?;

        let events = vec![mock_event1.clone()];
        client.expect_messages_to_l2_stream().returning(move |_| Ok(stream::iter(events.clone()).map(Ok).boxed()));
        client.expect_get_latest_block_number().returning(|| Ok(200));

        mock_canonical_block_hash(&mut client);
        mock_l1_handler_tx(&mut client, 1, true, false);
        client.expect_get_client_type().returning(|| ClientType::Eth);

        let client = Arc::new(client) as Arc<dyn SettlementLayerProvider>;
        let ctx_clone = ctx.clone();
        let db_backend_clone = db.clone();

        // Pass metadata_only = true (last argument)
        let sync_handle = tokio::spawn(async move { sync(client, db_backend_clone, notify, ctx, false, true).await });

        tokio::time::sleep(Duration::from_millis(500)).await;

        // Metadata SHOULD be written
        let l1_tx_hash = L1TransactionHash(mock_event1.l1_transaction_hash.to_be_bytes::<32>());
        assert_eq!(db.get_l1_txn_hash_by_nonce(mock_event1.message.tx.nonce).unwrap(), Some(l1_tx_hash));
        assert_eq!(
            db.get_messages_to_l2_by_l1_tx_hash(&l1_tx_hash).unwrap().unwrap(),
            vec![(mock_event1.message.tx.nonce, None)]
        );
        assert_eq!(
            db.get_l1_handler_l1_block_by_nonce(mock_event1.message.tx.nonce).unwrap(),
            Some(mock_event1.l1_block_number)
        );

        // Pending message should NOT be written
        assert!(
            db.get_pending_message_to_l2(mock_event1.message.tx.nonce).unwrap().is_none(),
            "Pending message should NOT be written when metadata_only is true"
        );

        // A consumer subscribed to the same notify should find nothing to consume
        let mut mock_for_consumer = MockSettlementLayerProvider::new();
        mock_for_consumer.expect_get_client_type().returning(|| ClientType::Eth);
        let consumer_notify = Arc::new(Notify::new());
        let mut consumer = MessagesToL2Consumer::new(db.clone(), Arc::new(mock_for_consumer), consumer_notify, false);
        assert!(
            consumer.consume_next_or_wait().now_or_never().is_none(),
            "Consumer should have nothing to consume when metadata_only is true"
        );

        ctx_clone.cancel_global();
        sync_handle.abort();
        Ok(())
    }
}
