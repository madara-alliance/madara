use crate::client::{ClientType, SettlementLayerProvider};
use crate::error::SettlementClientError;
use alloy::primitives::{B256, U256};
use futures::StreamExt;
use mc_db::MadaraBackend;
use mp_transactions::L1HandlerTransactionWithFee;
use mp_utils::service::ServiceContext;
use starknet_types_core::felt::Felt;
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;

mod find_start_block;

#[derive(Clone, Debug)]
pub struct MessageToL2WithMetadata {
    pub l1_block_number: u64,
    pub l1_transaction_hash: U256,
    pub message: L1HandlerTransactionWithFee,
}

/// Returns true if the message is valid, can be consumed.
pub async fn check_message_to_l2_validity(
    settlement_client: &Arc<dyn SettlementLayerProvider>,
    backend: &MadaraBackend,
    tx: &L1HandlerTransactionWithFee,
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

    if !settlement_client
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
) -> Result<(), SettlementClientError> {
    // sync inner is cancellation safe.
    ctx.run_until_cancelled(sync_inner(settlement_client, backend, notify_consumer)).await.transpose()?;
    Ok(())
}

async fn sync_inner(
    settlement_client: Arc<dyn SettlementLayerProvider>,
    backend: Arc<MadaraBackend>,
    notify_consumer: Arc<Notify>,
) -> Result<(), SettlementClientError> {
    // Note: Reprocessing events.
    // It's really important to make sure we don't mess up, we really want a strong guarantee we can't, in any circumstance, include an
    // l1 message that was already processed into a new block during sequencing.
    // Why? => if we do that, the state transition will be rejected when updating the core contract. That's really bad!
    // We can't make this guarantee here though. As such, we allow ourselves to reprocess events here, re-include them as pending & cie.
    // We still do *some* checks, but we can't make the full guarantee here. Instead, block production is responsible to make sure
    // it filters out any messages that are duplicated.
    // Thus, it's fine to reprocess some events :) there are caught during process message AND during block production.
    // In fact, we rely on that to restart sequencing on a clean database, or switch from full-node to sequencing.

    let chain_config = backend.chain_config();
    let replay_max_duration = chain_config.l1_messages_replay_max_duration;
    let finality_blocks = chain_config.l1_messages_finality_blocks;

    let from_l1_block_n = determine_start_block(&settlement_client, &backend, replay_max_duration).await?;

    tracing::info!("⟠  Starting L1 Messages Syncing from block #{from_l1_block_n} (finality: {finality_blocks} blocks)...");

    // Create the event stream
    let mut stream = settlement_client
        .messages_to_l2_stream(from_l1_block_n)
        .await
        .map_err(|e| SettlementClientError::StreamProcessing(format!("Failed to create messaging stream: {}", e)))?;

    // Buffer for events waiting for finality
    let mut pending_events: VecDeque<MessageToL2WithMetadata> = VecDeque::new();
    let mut stream_exhausted = false;

    loop {
        // Step 1: Receive new events from stream OR wait for finality check interval
        if !stream_exhausted {
            // Try to receive from stream with a short timeout
            let timeout = tokio::time::sleep(Duration::from_millis(100));
            tokio::select! {
                biased; // Prioritize receiving new events

                event = stream.next() => {
                    match event {
                        Some(Ok(msg)) => {
                            tracing::debug!(
                                "Received event from L1 block {}, tx {:#x}",
                                msg.l1_block_number,
                                msg.l1_transaction_hash
                            );
                            pending_events.push_back(msg);
                        }
                        Some(Err(e)) => {
                            tracing::warn!("Error from L1 event stream: {e:#}");
                        }
                        None => {
                            // Stream exhausted
                            tracing::debug!("L1 event stream exhausted");
                            stream_exhausted = true;
                        }
                    }
                }

                _ = timeout => {
                    // Timeout - proceed to check finality on pending events
                }
            }
        } else {
            // Stream exhausted - wait before checking for new blocks
            // This prevents busy-looping when caught up
            tokio::time::sleep(Duration::from_secs(12)).await;

            // Restart stream from current sync tip
            let current_tip = backend
                .get_l1_messaging_sync_tip()
                .map_err(|e| SettlementClientError::DatabaseError(format!("Failed to get sync tip: {}", e)))?
                .unwrap_or(from_l1_block_n);

            stream = settlement_client
                .messages_to_l2_stream(current_tip)
                .await
                .map_err(|e| {
                    SettlementClientError::StreamProcessing(format!("Failed to recreate messaging stream: {}", e))
                })?;
            stream_exhausted = false;
            tracing::debug!("Restarted L1 event stream from block {}", current_tip);
        }

        // Step 2: Process all finalized events from the front of the queue
        let processed_count = process_finalized_events(
            &settlement_client,
            &backend,
            &notify_consumer,
            &mut pending_events,
            finality_blocks,
        )
        .await?;

        if processed_count > 0 {
            tracing::debug!("Processed {} finalized events", processed_count);
        }
    }
}

/// Determines the L1 block to start syncing from.
async fn determine_start_block(
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

/// Processes events from the queue that have reached finality.
/// Returns the number of events processed.
async fn process_finalized_events(
    settlement_client: &Arc<dyn SettlementLayerProvider>,
    backend: &MadaraBackend,
    notify_consumer: &Notify,
    pending_events: &mut VecDeque<MessageToL2WithMetadata>,
    finality_blocks: u64,
) -> Result<usize, SettlementClientError> {
    if pending_events.is_empty() {
        return Ok(0);
    }

    // Get latest L1 block to check finality
    let latest_l1_block = settlement_client.get_latest_block_number().await?;
    let mut processed_count = 0;

    // Process events from the front (oldest first) that have reached finality
    while let Some(event) = pending_events.front() {
        let blocks_since_event = latest_l1_block.saturating_sub(event.l1_block_number);

        // Check if event has reached required finality
        if blocks_since_event < finality_blocks {
            tracing::debug!(
                "Event at L1 block {} not finalized yet ({} < {} blocks required)",
                event.l1_block_number,
                blocks_since_event,
                finality_blocks
            );
            break; // Events are ordered, so all remaining events are also not finalized
        }

        // Event is finalized - pop and process it
        let event = pending_events.pop_front().unwrap();

        tracing::debug!(
            "Processing finalized event from L1 block {}, tx {:#x}, from_address: {:#x}",
            event.l1_block_number,
            event.l1_transaction_hash,
            event.message.tx.calldata[0],
        );

        // Check validity and store if valid
        let is_valid = check_message_to_l2_validity(settlement_client, backend, &event.message)
            .await
            .map_err(|e| {
                SettlementClientError::InvalidResponse(format!(
                    "Checking validity for message in tx {}, block {}: {}",
                    event.l1_transaction_hash, event.l1_block_number, e
                ))
            })?;

        if is_valid
        {
            backend
                .write_pending_message_to_l2(&event.message)
                .map_err(|e| SettlementClientError::DatabaseError(format!("Adding l1 to l2 message to db: {}", e)))?;
        }

        // Update sync tip - only for finalized events
        backend.write_l1_messaging_sync_tip(Some(event.l1_block_number)).map_err(|e| {
            SettlementClientError::DatabaseError(format!("Failed to update l1 messaging sync tip: {}", e))
        })?;

        notify_consumer.notify_waiters();
        processed_count += 1;
    }

    Ok(processed_count)
}

#[cfg(test)]
mod messaging_module_tests {
    use super::*;
    use crate::client::MockSettlementLayerProvider;
    use futures::stream;
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
        client
            .expect_messages_to_l2_stream()
            .returning(move |_| Ok(stream::iter(events.clone()).map(Ok).boxed()));

        // Mock get_latest_block_number (needed for finality check)
        // Event is at block 100, latest is 200, so finality check passes (default finality_blocks=10)
        client.expect_get_latest_block_number().returning(|| Ok(200));

        // nonce 1, is pending, not being cancelled, not consumed in db. => OK
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
        let sync_handle = tokio::spawn(async move { sync(client, db_backend_clone, notify, ctx).await });

        // Wait for event to be processed (short wait since stream returns immediately)
        tokio::time::sleep(Duration::from_millis(500)).await;

        // Verify the message was processed
        assert_eq!(db.get_pending_message_to_l2(mock_event1.message.tx.nonce).unwrap().unwrap(), mock_event1.message);

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
        let sync_handle = tokio::spawn(async move { sync(client, db_backend_clone, notify, ctx).await });

        // Wait for event to be processed (short wait since stream returns immediately)
        tokio::time::sleep(Duration::from_millis(500)).await;

        // Verify the message was processed
        assert_eq!(db.get_pending_message_to_l2(mock_event1.message.tx.nonce).unwrap().unwrap(), mock_event1.message);

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
        mock_client
            .expect_messages_to_l2_stream()
            .returning(move |_| Ok(stream::iter(events.clone()).map(Ok).boxed()));

        // Latest block is 105 - only 5 blocks after event, less than 10 required
        // Event should NOT be processed yet
        mock_client.expect_get_latest_block_number().returning(|| Ok(105));

        mock_client.expect_get_client_type().returning(|| ClientType::Eth);

        let client = Arc::new(mock_client) as Arc<dyn SettlementLayerProvider>;
        let ctx = ServiceContext::new_for_testing();
        let ctx_clone = ctx.clone();
        let notify = Arc::new(Notify::new());
        let db_clone = db.clone();

        let sync_handle = tokio::spawn(async move { sync(client, db_clone, notify, ctx).await });

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
        mock_client
            .expect_messages_to_l2_stream()
            .returning(move |_| Ok(stream::iter(events.clone()).map(Ok).boxed()));

        // Latest block is 115 - 15 blocks after event, more than 10 required
        // Event SHOULD be processed
        mock_client.expect_get_latest_block_number().returning(|| Ok(115));

        mock_client.expect_get_client_type().returning(|| ClientType::Eth);
        mock_l1_handler_tx(&mut mock_client, 1, true, false);

        let client = Arc::new(mock_client) as Arc<dyn SettlementLayerProvider>;
        let ctx = ServiceContext::new_for_testing();
        let ctx_clone = ctx.clone();
        let notify = Arc::new(Notify::new());
        let db_clone = db.clone();

        let sync_handle = tokio::spawn(async move { sync(client, db_clone, notify, ctx).await });

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
}
