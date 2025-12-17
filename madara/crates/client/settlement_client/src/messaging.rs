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

/// Interval for polling the stream and checking finality on queued events.
const STREAM_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Base delay for reconnection attempts after stream failure.
const RECONNECT_BASE_DELAY: Duration = Duration::from_secs(1);

/// Maximum delay between reconnection attempts (exponential backoff cap).
const RECONNECT_MAX_DELAY: Duration = Duration::from_secs(60);

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
    // Note: It's fine to reprocess events - duplicates are filtered during block production.

    let chain_config = backend.chain_config();
    let replay_max_duration = chain_config.l1_messages_replay_max_duration;
    let finality_blocks = chain_config.l1_messages_finality_blocks;

    let mut reconnect_delay = RECONNECT_BASE_DELAY;

    loop {
        match run_message_sync(&settlement_client, &backend, &notify_consumer, replay_max_duration, finality_blocks)
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
        if let Err(e) =
            process_finalized_events(settlement_client, backend, notify_consumer, &mut pending_events, finality_blocks)
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

/// Processes events from the queue that have reached the required finality threshold.
async fn process_finalized_events(
    settlement_client: &Arc<dyn SettlementLayerProvider>,
    backend: &MadaraBackend,
    notify_consumer: &Notify,
    pending_events: &mut VecDeque<MessageToL2WithMetadata>,
    finality_blocks: u64,
) -> Result<(), SettlementClientError> {
    if pending_events.is_empty() {
        return Ok(());
    }

    let latest_l1_block = settlement_client.get_latest_block_number().await?;

    // Process events from the front (oldest first) that have reached finality
    while let Some(event) = pending_events.front() {
        let confirmations = latest_l1_block.saturating_sub(event.l1_block_number);

        if confirmations < finality_blocks {
            tracing::debug!(
                "Message at block {} waiting for finality: {}/{} confirmations",
                event.l1_block_number,
                confirmations,
                finality_blocks
            );
            break; // Events are ordered, remaining events are also not finalized
        }

        // SAFETY: We just confirmed front() returned Some
        let event = pending_events.pop_front().expect("front() was Some");

        tracing::info!(
            "Processing L1→L2 message: block={}, nonce={}, confirmations={}",
            event.l1_block_number,
            event.message.tx.nonce,
            confirmations
        );

        let is_valid = check_message_to_l2_validity(settlement_client, backend, &event.message).await.map_err(|e| {
            SettlementClientError::InvalidResponse(format!(
                "Validity check failed for tx {}: {}",
                event.l1_transaction_hash, e
            ))
        })?;

        if is_valid {
            backend
                .write_pending_message_to_l2(&event.message)
                .map_err(|e| SettlementClientError::DatabaseError(format!("Failed to store message: {}", e)))?;
            tracing::debug!("Message stored: nonce={}", event.message.tx.nonce);
        } else {
            tracing::debug!("Message skipped (invalid/cancelled): nonce={}", event.message.tx.nonce);
        }

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
        client.expect_messages_to_l2_stream().returning(move |_| Ok(stream::iter(events.clone()).map(Ok).boxed()));

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
        mock_client.expect_messages_to_l2_stream().returning(move |_| Ok(stream::iter(events.clone()).map(Ok).boxed()));

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
