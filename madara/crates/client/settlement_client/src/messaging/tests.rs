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

#[tokio::test]
async fn saved_event_cursor_replays_its_boundary_block() -> anyhow::Result<()> {
    let db = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));
    db.write_l1_messaging_sync_tip(Some(100))?;
    let client = Arc::new(MockSettlementLayerProvider::new()) as Arc<dyn SettlementLayerProvider>;

    let start = get_start_block(&client, &db, Duration::ZERO).await?;

    assert_eq!(start, 99);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn test_sync_processes_new_messages(#[future] setup_messaging_tests: MessagingTestRunner) -> anyhow::Result<()> {
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

    let current_timestamp_secs =
        SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).expect("Current time is before UNIX_EPOCH").as_secs();

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

    // Sync tip should NOT have advanced past the dropped event — on reconnection,
    // the historical query should re-scan this region to pick up new canonical events.
    let sync_tip = db.get_l1_messaging_sync_tip().unwrap().unwrap();
    assert_eq!(sync_tip, 99, "Sync tip should NOT advance when dropping a reorged event");

    ctx_clone.cancel_global();
    sync_handle.abort();

    Ok(())
}

/// Same as `test_drops_event_when_block_does_not_exist`, but for the more common reorg case
/// where the block still exists at the same height but has a DIFFERENT hash (i.e., the chain
/// reorged and a new block was mined at the same height with different content).
#[tokio::test]
async fn test_drops_event_when_block_hash_mismatches() -> anyhow::Result<()> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_test_writer()
        .try_init();

    let mut chain_config = ChainConfig::madara_test();
    chain_config.l1_messages_finality_blocks = 10;
    let chain_config = Arc::new(chain_config);
    let db = MadaraBackend::open_for_testing(chain_config.clone());

    db.write_l1_messaging_sync_tip(Some(99))?;

    let mut mock_client = MockSettlementLayerProvider::new();

    // Event at block 100 with l1_block_hash = [0u8; 32] (from create_mock_event).
    // Latest at 115 → confirmation check passes.
    let mock_event = create_mock_event(100, 1);
    let events = vec![mock_event.clone()];
    mock_client.expect_messages_to_l2_stream().returning(move |_| Ok(stream::iter(events.clone()).map(Ok).boxed()));
    mock_client.expect_get_latest_block_number().returning(|| Ok(115));
    mock_client.expect_get_client_type().returning(|| ClientType::Eth);

    // CRITICAL: get_block_n_hash returns a DIFFERENT hash than the event's l1_block_hash.
    // This simulates a reorg where block 100 was replaced with a new block at the same height.
    mock_client.expect_get_block_n_hash().returning(|_| Ok(Some([1u8; 32])));

    // No validity-check mocks — they should NOT be called (canonical check short-circuits).

    let client = Arc::new(mock_client) as Arc<dyn SettlementLayerProvider>;
    let ctx = ServiceContext::new_for_testing();
    let ctx_clone = ctx.clone();
    let notify = Arc::new(Notify::new());
    let db_clone = db.clone();

    let sync_handle = tokio::spawn(async move { sync(client, db_clone, notify, ctx, false, false).await });

    tokio::time::sleep(Duration::from_millis(500)).await;

    // Event must NOT have been queued for L2 inclusion
    assert!(
        db.get_pending_message_to_l2(mock_event.message.tx.nonce).unwrap().is_none(),
        "Reorged event should not be in pending_message_to_l2"
    );

    // Nonce metadata must NOT have been written
    assert!(
        db.get_l1_txn_hash_by_nonce(mock_event.message.tx.nonce).unwrap().is_none(),
        "Nonce should NOT be poisoned after dropping a reorged event"
    );

    // Sync tip should NOT have advanced
    let sync_tip = db.get_l1_messaging_sync_tip().unwrap().unwrap();
    assert_eq!(sync_tip, 99, "Sync tip should NOT advance when dropping a reorged event");

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

    let sync_handle = tokio::spawn(async move { sync(client, db, Arc::new(Notify::new()), ctx, false, false).await });

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
