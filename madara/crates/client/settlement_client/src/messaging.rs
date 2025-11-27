use crate::client::{ClientType, SettlementLayerProvider};
use crate::error::SettlementClientError;
use alloy::primitives::{B256, U256};
use anyhow::Context;
use futures::{StreamExt, TryStreamExt};
use mc_db::MadaraBackend;
use mp_transactions::L1HandlerTransactionWithFee;
use mp_utils::service::ServiceContext;
use starknet_types_core::felt::Felt;
use std::sync::Arc;
use tokio::sync::Notify;

pub mod depth_filtered_stream;
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
    l1_msg_min_confirmations: u64,
    block_poll_interval: std::time::Duration,
) -> Result<(), SettlementClientError> {
    // sync inner is cancellation safe.
    ctx.run_until_cancelled(sync_inner(
        settlement_client,
        backend,
        notify_consumer,
        l1_msg_min_confirmations,
        block_poll_interval,
    ))
    .await
    .transpose()?;
    Ok(())
}

async fn sync_inner(
    settlement_client: Arc<dyn SettlementLayerProvider>,
    backend: Arc<MadaraBackend>,
    notify_consumer: Arc<Notify>,
    l1_msg_min_confirmations: u64,
    block_poll_interval: std::time::Duration,
) -> Result<(), SettlementClientError> {
    // Note: Reprocessing events.
    // It's really important to make sure we don't mess up, we really want a strong guarantee we can't, in any circumstance, include an
    // l1 message that was already processed into a new block during sequencing.
    // Why? => if we do that, the state transition will be rejected when updating the core contract. That's really bad!
    // We can't make this guarantee here though. As such, we allow ourselves to reprocess events here, re-include them as pending & cie.
    // We still do *some* checks, but we can't make the full guarantee here. Instead, block production is responsible to make sure
    // it filters out any messages that are duplicated.
    // Thus, it's fine to reprocess some events :) they are caught during process message AND during block production.
    // In fact, we rely on that to restart sequencing on a clean database, or switch from full-node to sequencing.

    let replay_max_duration = backend.chain_config().l1_messages_replay_max_duration;

    let from_l1_block_n = if let Some(block_n) = backend
        .get_l1_messaging_sync_tip()
        .map_err(|e| SettlementClientError::DatabaseError(format!("Failed to get last synced event block: {}", e)))?
    {
        block_n
    } else if !replay_max_duration.is_zero() {
        tracing::debug!("Getting latest block_n from settlement layer.");
        let latest_block_n = settlement_client.get_latest_block_number().await?;
        tracing::debug!("Find start, latest {latest_block_n}...");
        find_start_block::find_replay_block_n_start(&settlement_client, replay_max_duration, latest_block_n).await?
    } else {
        settlement_client.get_latest_block_number().await?
    };

    tracing::info!("⟠  Starting L1 Messages Syncing from block #{from_l1_block_n}...");

    settlement_client
        .messages_to_l2_stream(from_l1_block_n, l1_msg_min_confirmations, block_poll_interval)
        .await
        .map_err(|e| SettlementClientError::StreamProcessing(format!("Failed to create messaging stream: {}", e)))?
        .map(|message| {
            let (backend, settlement_client) = (backend.clone(), settlement_client.clone());
            let fut = async move {
                let message = message?;
                tracing::debug!(
                    "Processing Message from block: {:?}, transaction_hash: {:#x}, fromAddress: {:#x}",
                    message.l1_block_number,
                    message.l1_transaction_hash,
                    message.message.tx.calldata[0],
                );

                if check_message_to_l2_validity(&settlement_client, &backend, &message.message).await
                    .with_context(|| format!("Checking validity for message in {}, {}", message.l1_transaction_hash, message.l1_block_number))? {
                    // Add the pending message to db.
                    backend
                        .write_pending_message_to_l2(&message.message)
                        .map_err(|e| SettlementClientError::DatabaseError(format!("Adding l1 to l2 message to db: {}", e)))?;
                }
                anyhow::Ok((message.l1_transaction_hash, message.l1_block_number))
            };
            async move {
                Ok(fut.await)
            }
        })
        .buffered(/* concurrency */ 5) // add a bit of concurrency to speed up the catch up time if needed.
        // sequentially, update l1_messaging_sync_tip
        .try_for_each(|block_n| {
            let backend = backend.clone();
            let notify_consumer = notify_consumer.clone();
            async move {
                match block_n {
                    Err(err) => tracing::debug!("Error while parsing the next ethereum message: {err:#}"),
                    Ok((tx_hash, block_n)) => {
                        tracing::debug!("Processed {tx_hash:#x} {block_n}");
                        tracing::debug!("Set l1_messaging_sync_tip={block_n}");
                        backend.write_l1_messaging_sync_tip(Some(block_n)).map_err(|e| {
                            SettlementClientError::DatabaseError(format!("Failed to get last synced event block: {}", e))
                        })?;
                        notify_consumer.notify_waiters(); // notify
                    }
                }
                Ok(())
            }
        })
        .await
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

        // Setup mock for last synced block
        db.write_l1_messaging_sync_tip(Some(99))?;

        // Mock get_messaging_stream
        let events = vec![mock_event1.clone()];
        client
            .expect_messages_to_l2_stream()
            .times(1)
            .returning(move |_, _, _| Ok(stream::iter(events.clone()).map(Ok).boxed()));

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
        let sync_handle =
            tokio::spawn(async move { sync(client, db_backend_clone, notify, ctx, 0, Duration::from_secs(12)).await });

        // Wait sufficient time for event to be processed
        tokio::time::sleep(Duration::from_secs(5)).await;

        // Verify the message was processed

        // nonce 1, is pending, not being cancelled, not consumed in db. => OK
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
            .with(predicate::eq(from_l1_block_n), predicate::always(), predicate::always())
            .times(1)
            .returning(move |_, _, _| Ok(stream::iter(events.clone()).map(Ok).boxed()));

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
        let sync_handle =
            tokio::spawn(async move { sync(client, db_backend_clone, notify, ctx, 0, Duration::from_secs(12)).await });

        // Wait sufficient time for event to be processed
        tokio::time::sleep(Duration::from_secs(5)).await;

        // Verify the message was processed

        // nonce 1, is pending, not being cancelled, not consumed in db. => OK
        assert_eq!(db.get_pending_message_to_l2(mock_event1.message.tx.nonce).unwrap().unwrap(), mock_event1.message);
        // Clean up: cancel context and abort task
        ctx_clone.cancel_global();
        sync_handle.abort();

        Ok(())
    }
}
