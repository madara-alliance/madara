use crate::client::{ClientType, SettlementClientTrait};
use crate::error::SettlementClientError;
use alloy::primitives::{B256, U256};
use futures::{StreamExt, TryStreamExt};
use mc_db::MadaraBackend;
use mp_transactions::L1HandlerTransactionWithFee;
use mp_utils::service::ServiceContext;
use starknet_types_core::felt::Felt;
use tokio::sync::Notify;
use std::sync::Arc;

mod find_start_block;

pub struct MessageToL2WithMetadata {
    pub l1_block_number: u64,
    pub l1_transaction_hash: U256,
    pub message: L1HandlerTransactionWithFee,
}

/// Returns true if the message is valid, can be consumed.
pub async fn check_message_to_l2_validity(
    settlement_client: &Arc<dyn SettlementClientTrait>,
    backend: &MadaraBackend,
    tx: &L1HandlerTransactionWithFee,
) -> Result<bool, SettlementClientError> {
    // Skip if already processed.
    if backend
        .get_l1_handler_txn_hash_by_core_contract_nonce(tx.tx.nonce)
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
    let event_hash = settlement_client.get_messaging_hash(&tx)?;
    let converted_event_hash = match settlement_client.get_client_type() {
        ClientType::Eth => B256::from_slice(event_hash.as_slice()).to_string(),
        ClientType::Starknet => Felt::from_bytes_be_slice(event_hash.as_slice()).to_hex_string(),
    };
    tracing::debug!("Checking for cancellation, event hash: {:?}", converted_event_hash);

    if !settlement_client
        .check_l1_to_l2_message_exists(&event_hash)
        .await
        .map_err(|e| SettlementClientError::InvalidResponse(format!("Failed to check message still exists: {}", e)))?
    {
        tracing::debug!("Message does not exist anymore in core contract.");
        return Ok(false);
    }

    let cancellation_timestamp = settlement_client
        .get_l1_to_l2_message_cancellation(&event_hash)
        .await
        .map_err(|e| SettlementClientError::InvalidResponse(format!("Failed to check cancellation: {}", e)))?;
    if cancellation_timestamp != Felt::ZERO {
        tracing::debug!("Message was cancelled in block at timestamp: {:?}", cancellation_timestamp);
        return Ok(false);
    }

    Ok(true)
}

pub async fn sync(
    settlement_client: Arc<dyn SettlementClientTrait>,
    backend: Arc<MadaraBackend>,
    notify_consumer: Arc<Notify>,
    mut ctx: ServiceContext,
) -> Result<(), SettlementClientError> {
    // sync inner is cancellation safe.
    ctx.run_until_cancelled(sync_inner(settlement_client, backend, notify_consumer)).await.transpose()?;
    Ok(())
}

async fn sync_inner(
    settlement_client: Arc<dyn SettlementClientTrait>,
    backend: Arc<MadaraBackend>,
    notify_consumer: Arc<Notify>,
) -> Result<(), SettlementClientError> {
    // Note: Reprocessing events.
    // It's really important to make sure we don't mess up, we really want a strong guarantee we can't, in any circonstance, include an
    // l1 message that was already processed into a new block during sequencing.
    // Why? => if we do that, the state transition will be rejected when updating the core contract. That's really bad!
    // We can't make this guarantee here though. As such, we allow ourselves to reprocess events here, re-include them as pending & cie.
    // We still do *some* checks, but we can't make the full guarantee here. Instead, block production is responsible to make sure
    // it filters out any messages that are duplicated.
    // Thus, it's fine to reprocess some events :) there are caught during process message AND during block production.
    // In fact, we rely on that to restart sequencing on a clean database, or switch from full-node to sequencing.

    let replay_max_duration = backend.chain_config().l1_messages_replay_max_duration;

    let from_l1_block_n = if let Some(block_n) = backend
        .get_l1_messaging_sync_tip()
        .map_err(|e| SettlementClientError::DatabaseError(format!("Failed to get last synced event block: {}", e)))?
    {
        block_n
    } else {
        let latest_block_n = settlement_client.get_latest_block_number().await?;
        find_start_block::find_replay_block_n_start(&settlement_client, replay_max_duration, latest_block_n).await?
    };

    tracing::info!("⟠  Starting L1 Messages Syncing from block #{from_l1_block_n}...");

    settlement_client
        .l1_to_l2_messages_stream(from_l1_block_n, None)
        .await
        .map_err(|e| SettlementClientError::StreamProcessing(format!("Failed to create messaging stream: {}", e)))?
        .map(|message| {
            let (backend, settlement_client) = (backend.clone(), settlement_client.clone());
            async move {
                // TODO: fix this case!
                // // If the message has felts that are out of range, conversion will fail. We shouldn't quit the worker because of that.
                // if let Err(SettlementClientError::ConversionError(e)) = message {
                //     tracing::error!("Invalid message to l2: {e}");
                //     return Ok(None)
                // }
                let message = message?;
                tracing::debug!(
                    "Processing Message from block: {:?}, transaction_hash: {:#x}, fromAddress: {:#x}",
                    message.l1_block_number,
                    message.l1_transaction_hash,
                    message.message.tx.calldata[0],
                );
            
                if check_message_to_l2_validity(&settlement_client, &backend, &message.message).await? {
                    // Add the pending message to db.
                    backend
                        .add_pending_message_to_l2(message.message)
                        .map_err(|e| SettlementClientError::DatabaseError(format!("Adding l1 to l2 message to db: {}", e)))?;
                }
                Ok(message.l1_block_number)
            }
        })
        .buffered(/* concurrency */ 5) // add a bit of concurrency to speed up the catch up time if needed.
        // sequentially, update l1_messaging_sync_tip
        .try_for_each(|block_n| {
            let backend = backend.clone();
            let notify_consumer = notify_consumer.clone();
            async move {
                tracing::debug!("Set l1_messaging_sync_tip={block_n}");
                backend.set_l1_messaging_sync_tip(block_n).map_err(|e| {
                    SettlementClientError::DatabaseError(format!("Failed to get last synced event block: {}", e))
                })?;
                notify_consumer.notify_one();
                Ok(())
            }
        })
        .await
}

// #[cfg(test)]
// mod messaging_module_tests {
//     use super::*;
//     use crate::client::{
//         test_types::{DummyConfig, DummyStream},
//         MockSettlementClientTrait,
//     };
//     use futures::stream;
//     use mc_db::DatabaseService;
//     use mc_mempool::{Mempool, MempoolConfig};
//     use mp_chain_config::ChainConfig;
//     use rstest::{fixture, rstest};
//     use starknet_types_core::felt::Felt;
//     use std::time::Duration;

//     // Helper function to create a mock event
//     fn create_mock_event(block_number: u64, nonce: u64) -> L1toL2MessagingEventData {
//         L1toL2MessagingEventData {
//             block_number,
//             transaction_hash: Felt::from(1),
//             event_index: Some(0),
//             from: Felt::from(123),
//             to: Felt::from(456),
//             selector: Felt::from(789),
//             payload: vec![Felt::from(1), Felt::from(2)],
//             nonce: Felt::from(nonce),
//             fee: Some(1000),
//             message_hash: None,
//         }
//     }

//     struct MessagingTestRunner {
//         client: MockSettlementClientTrait,
//         db: Arc<DatabaseService>,
//         mempool: Arc<Mempool>,
//         ctx: ServiceContext,
//     }

//     #[fixture]
//     async fn setup_messaging_tests() -> MessagingTestRunner {
//         // Set up chain info
//         let chain_config = Arc::new(ChainConfig::madara_test());

//         // Initialize database service
//         let db = Arc::new(DatabaseService::open_for_testing(chain_config.clone()));

//         let mempool = Arc::new(Mempool::new(Arc::clone(db.backend()), MempoolConfig::for_testing()));

//         // Create a mock client directly
//         let mut mock_client = MockSettlementClientTrait::default();

//         // Configure basic mock expectations that all tests will need
//         mock_client.expect_get_client_type().returning(|| ClientType::Eth);

//         // Create a new service context for testing
//         let ctx = ServiceContext::new_for_testing();

//         MessagingTestRunner { client: mock_client, db, mempool, ctx }
//     }

//     #[rstest]
//     #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
//     async fn test_sync_processes_new_message(
//         #[future] setup_messaging_tests: MessagingTestRunner,
//     ) -> anyhow::Result<()> {
//         let MessagingTestRunner { mut client, db, mempool, ctx } = setup_messaging_tests.await;

//         // Setup mock event and configure backend
//         let mock_event = create_mock_event(100, 1);
//         let event_clone = mock_event.clone();
//         let backend = db.backend();

//         // Setup mock for last synced block
//         backend.messaging_update_last_synced_l1_block_with_event(LastSyncedEventBlock::new(99, 0))?;

//         // Mock get_messaging_stream
//         client
//             .expect_get_messaging_stream()
//             .times(1)
//             .returning(move |_| Ok(Box::pin(stream::iter(vec![Ok(mock_event.clone())]))));

//         // Mock get_messaging_hash
//         client.expect_get_messaging_hash().times(1).returning(|_| Ok(vec![0u8; 32]));

//         // Mock get_l1_to_l2_message_cancellations
//         client.expect_get_l1_to_l2_message_cancellations().times(1).returning(|_| Ok(Felt::ZERO));

//         // Mock get_client_type
//         client.expect_get_client_type().returning(|| ClientType::Eth);

//         // Wrap the client in Arc
//         let client = Arc::new(client) as Arc<dyn SettlementClientTrait<Config = DummyConfig, StreamType = DummyStream>>;

//         // Keep a reference to context for cancellation
//         let ctx_clone = ctx.clone();
//         let db_backend_clone = backend.clone();

//         // Spawn the sync task in a separate thread
//         let sync_handle = tokio::spawn(async move { sync(client, db_backend_clone, mempool.clone(), ctx).await });

//         // Wait sufficient time for event to be processed
//         tokio::time::sleep(Duration::from_secs(5)).await;

//         // Verify the message was processed
//         assert!(backend.has_l1_messaging_nonce(Nonce(event_clone.nonce))?);

//         // Clean up: cancel context and abort task
//         ctx_clone.cancel_global();
//         sync_handle.abort();

//         Ok(())
//     }

//     #[rstest]
//     #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
//     async fn test_sync_handles_cancelled_message(
//         #[future] setup_messaging_tests: MessagingTestRunner,
//     ) -> anyhow::Result<()> {
//         let MessagingTestRunner { mut client, db, mempool, ctx } = setup_messaging_tests.await;

//         let backend = db.backend();

//         // Setup mock event
//         let mock_event = create_mock_event(100, 1);
//         let event_clone = mock_event.clone();

//         // Setup mock for last synced block
//         backend.messaging_update_last_synced_l1_block_with_event(LastSyncedEventBlock::new(99, 0))?;

//         // Mock get_messaging_stream
//         client
//             .expect_get_messaging_stream()
//             .times(1)
//             .returning(move |_| Ok(Box::pin(stream::iter(vec![Ok(mock_event.clone())]))));

//         // Mock get_messaging_hash
//         client.expect_get_messaging_hash().times(1).returning(|_| Ok(vec![0u8; 32]));

//         // Mock get_l1_to_l2_message_cancellations - return non-zero to indicate cancellation
//         client.expect_get_l1_to_l2_message_cancellations().times(1).returning(|_| Ok(Felt::from(12345)));

//         // Wrap the client in Arc
//         let client = Arc::new(client) as Arc<dyn SettlementClientTrait<Config = DummyConfig, StreamType = DummyStream>>;

//         // Keep a reference to context for cancellation
//         let ctx_clone = ctx.clone();
//         let db_backend_clone = backend.clone();

//         // Spawn the sync task in a separate thread
//         let sync_handle = tokio::spawn(async move { sync(client, db_backend_clone, mempool.clone(), ctx).await });

//         // Wait sufficient time for event to be processed
//         tokio::time::sleep(Duration::from_secs(5)).await;

//         // Verify the cancelled message was handled correctly
//         assert!(backend.has_l1_messaging_nonce(Nonce(event_clone.nonce))?);

//         // Clean up: cancel context and abort task
//         ctx_clone.cancel_global();
//         sync_handle.abort();

//         Ok(())
//     }

//     #[rstest]
//     #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
//     async fn test_sync_skips_already_processed_message(
//         #[future] setup_messaging_tests: MessagingTestRunner,
//     ) -> anyhow::Result<()> {
//         let MessagingTestRunner { mut client, db, mempool, ctx } = setup_messaging_tests.await;

//         let backend = db.backend();

//         // Setup mock event
//         let mock_event = create_mock_event(100, 1);

//         // Pre-set the nonce as processed
//         backend.set_l1_messaging_nonce(Nonce(mock_event.nonce))?;

//         // Setup mock for last synced block
//         backend.messaging_update_last_synced_l1_block_with_event(LastSyncedEventBlock::new(99, 0))?;

//         // Mock get_messaging_stream
//         client
//             .expect_get_messaging_stream()
//             .times(1)
//             .returning(move |_| Ok(Box::pin(stream::iter(vec![Ok(mock_event.clone())]))));

//         // Mock get_messaging_hash - should not be called
//         client.expect_get_messaging_hash().times(0);

//         // Wrap the client in Arc
//         let client = Arc::new(client) as Arc<dyn SettlementClientTrait<Config = DummyConfig, StreamType = DummyStream>>;

//         // Keep a reference to context for cancellation
//         let ctx_clone = ctx.clone();
//         let db_backend_clone = backend.clone();

//         // Spawn the sync task in a separate thread
//         let sync_handle = tokio::spawn(async move { sync(client, db_backend_clone, mempool.clone(), ctx).await });

//         // Wait sufficient time for event to be processed
//         tokio::time::sleep(Duration::from_secs(5)).await;

//         // Clean up: cancel context and abort task
//         ctx_clone.cancel_global();
//         sync_handle.abort();

//         Ok(())
//     }

//     #[rstest]
//     #[tokio::test]
//     async fn test_sync_handles_stream_errors(
//         #[future] setup_messaging_tests: MessagingTestRunner,
//     ) -> anyhow::Result<()> {
//         let MessagingTestRunner { mut client, db, mempool, ctx } = setup_messaging_tests.await;

//         let backend = db.backend();

//         // Setup mock for last synced block
//         backend.messaging_update_last_synced_l1_block_with_event(LastSyncedEventBlock::new(99, 0))?;

//         // Mock get_messaging_stream to return error
//         client.expect_get_messaging_stream().times(1).returning(move |_| {
//             Ok(Box::pin(stream::iter(vec![Err(SettlementClientError::Other("Stream error".to_string()))])))
//         });

//         let client: Arc<dyn SettlementClientTrait<Config = DummyConfig, StreamType = DummyStream>> = Arc::new(client);

//         let result = sync(client, backend.clone(), mempool.clone(), ctx).await;
//         assert!(result.is_err());
//         assert!(result.unwrap_err().to_string().contains("Stream error"));

//         Ok(())
//     }
// }
