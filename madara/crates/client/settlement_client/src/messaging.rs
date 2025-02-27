use crate::client::{ClientType, SettlementClientTrait};
use crate::error::SettlementClientError;
use alloy::primitives::B256;
use futures::{Stream, StreamExt};
use mc_db::l1_db::LastSyncedEventBlock;
use mc_db::MadaraBackend;
use mc_mempool::{Mempool, MempoolProvider};
use mp_utils::service::ServiceContext;
use starknet_api::core::{ContractAddress, EntryPointSelector, Nonce};
use starknet_api::transaction::{Calldata, L1HandlerTransaction, TransactionVersion};
use starknet_types_core::felt::Felt;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct L1toL2MessagingEventData {
    pub from: Felt,
    pub to: Felt,
    pub selector: Felt,
    pub nonce: Felt,
    pub payload: Vec<Felt>,
    pub fee: Option<u128>,
    pub transaction_hash: Felt,
    pub message_hash: Option<Felt>,
    pub block_number: u64,
    pub event_index: Option<u64>,
}

pub async fn sync<C, S>(
    settlement_client: Arc<dyn SettlementClientTrait<Config = C, StreamType = S>>,
    backend: Arc<MadaraBackend>,
    mempool: Arc<Mempool>,
    mut ctx: ServiceContext,
) -> Result<(), SettlementClientError>
where
    S: Stream<Item = Result<L1toL2MessagingEventData, SettlementClientError>> + Send + 'static,
{
    tracing::info!("⟠ Starting L1 Messages Syncing...");

    let last_synced_event_block = backend
        .messaging_last_synced_l1_block_with_event()
        .map_err(|e| SettlementClientError::Other(e.to_string()))?
        .ok_or_else(|| SettlementClientError::Other("Last synced event block should never be None".to_string()))?;

    let stream = settlement_client
        .get_messaging_stream(last_synced_event_block)
        .await
        .map_err(|e| SettlementClientError::Other(e.to_string()))?;
    let mut event_stream = Box::pin(stream);

    while let Some(event_result) = ctx.run_until_cancelled(event_stream.next()).await {
        if let Some(event) = event_result {
            let event_data = event?;
            let tx = parse_handle_message_transaction(&event_data)
                .map_err(|e| SettlementClientError::Other(e.to_string()))?;
            let tx_nonce = tx.nonce;

            // Skip if already processed
            if backend.has_l1_messaging_nonce(tx_nonce).map_err(|e| SettlementClientError::Other(e.to_string()))? {
                tracing::info!("Event already processed");
                return Ok(());
            }

            tracing::info!(
                "Processing Message from block: {:?}, transaction_hash: {:?}, fromAddress: {:?}",
                event_data.block_number,
                format!("{}", event_data.transaction_hash.to_hex_string()),
                format!("{}", event_data.from.to_hex_string()),
            );

            // Check message hash and cancellation
            let event_hash = settlement_client.get_messaging_hash(&event_data)?;
            let converted_event_hash = match settlement_client.get_client_type() {
                ClientType::ETH => B256::from_slice(event_hash.as_slice()).to_string(),
                ClientType::STARKNET => Felt::from_bytes_be_slice(event_hash.as_slice()).to_hex_string(),
            };
            tracing::info!("Checking for cancellation, event hash: {:?}", converted_event_hash);

            let cancellation_timestamp = settlement_client
                .get_l1_to_l2_message_cancellations(&event_hash)
                .await
                .map_err(|e| SettlementClientError::Other(e.to_string()))?;
            if cancellation_timestamp != Felt::ZERO {
                tracing::info!("Message was cancelled in block at timestamp: {:?}", cancellation_timestamp);
                handle_cancelled_message(backend, tx_nonce).map_err(|e| SettlementClientError::Other(e.to_string()))?;
                return Ok(());
            }

            // Process message
            match process_message(&backend, &event_data, mempool.clone()).await {
                Ok(Some(tx_hash)) => {
                    tracing::info!(
                        "Message from block: {:?} submitted, transaction hash: {:?}",
                        event_data.block_number,
                        tx_hash
                    );

                    let block_sent =
                        LastSyncedEventBlock::new(event_data.block_number, event_data.event_index.unwrap_or(0));
                    backend
                        .messaging_update_last_synced_l1_block_with_event(block_sent)
                        .map_err(|e| SettlementClientError::Other(e.to_string()))?;
                    backend
                        .set_l1_messaging_nonce(tx_nonce)
                        .map_err(|e| SettlementClientError::Other(e.to_string()))?;
                }
                Ok(None) => {
                    tracing::info!("Message from block: {:?} skipped (already processed)", event_data.block_number);
                }
                Err(e) => {
                    tracing::error!(
                        "Unexpected error while processing Message from block: {:?}, error: {:?}",
                        event_data.block_number,
                        e
                    );
                    return Err(SettlementClientError::Other(format!("Failed to process message: {}", e)));
                }
            }
        }
    }

    Ok(())
}

fn handle_cancelled_message(backend: Arc<MadaraBackend>, nonce: Nonce) -> Result<(), SettlementClientError> {
    match backend.has_l1_messaging_nonce(nonce) {
        Ok(false) => {
            backend.set_l1_messaging_nonce(nonce).map_err(|e| SettlementClientError::Other(e.to_string()))?;
        }
        Ok(true) => {}
        Err(e) => {
            tracing::error!("Unexpected DB error: {:?}", e);
            return Err(SettlementClientError::Other(format!("Database error: {}", e)));
        }
    }
    Ok(())
}

pub fn parse_handle_message_transaction(
    event: &L1toL2MessagingEventData,
) -> Result<L1HandlerTransaction, SettlementClientError> {
    let calldata =
        Calldata(Arc::new(std::iter::once(event.from).chain(event.payload.iter().cloned()).collect::<Vec<_>>()));

    Ok(L1HandlerTransaction {
        nonce: Nonce(event.nonce),
        contract_address: ContractAddress(
            event
                .to
                .try_into()
                .map_err(|_| SettlementClientError::Other("Failed to convert to contract address".to_string()))?,
        ),
        entry_point_selector: EntryPointSelector(event.selector),
        calldata,
        version: TransactionVersion(Felt::ZERO),
    })
}

async fn process_message(
    backend: &MadaraBackend,
    event: &L1toL2MessagingEventData,
    mempool: Arc<Mempool>,
) -> Result<Option<Felt>, SettlementClientError> {
    let transaction = parse_handle_message_transaction(event)?;
    let tx_nonce = transaction.nonce;
    let fees = event.fee;

    // Ensure that L1 message has not been executed
    match backend.has_l1_messaging_nonce(tx_nonce) {
        Ok(false) => {
            backend.set_l1_messaging_nonce(tx_nonce).map_err(|e| SettlementClientError::Other(e.to_string()))?;
        }
        Ok(true) => {
            tracing::debug!("⟠ Event already processed: {:?}", transaction);
            return Ok(None);
        }
        Err(e) => {
            tracing::error!("⟠ Unexpected DB error: {:?}", e);
            return Err(SettlementClientError::Other(format!("Database error: {}", e)));
        }
    };

    let res = mempool
        .tx_accept_l1_handler(transaction.into(), fees.unwrap_or(0))
        .map_err(|e| SettlementClientError::Other(e.to_string()))?;

    Ok(Some(res.transaction_hash))
}

#[cfg(test)]
mod messaging_module_tests {
    use super::*;
    use crate::client::{
        test_types::{DummyConfig, DummyStream},
        MockSettlementClientTrait,
    };
    use futures::stream;
    use mc_db::DatabaseService;
    use mc_mempool::{GasPriceProvider, L1DataProvider, MempoolLimits};
    use mp_chain_config::ChainConfig;
    use rstest::{fixture, rstest};
    use starknet_types_core::felt::Felt;
    use std::time::Duration;
    use tokio::time::timeout;

    // Helper function to create a mock event
    fn create_mock_event(block_number: u64, nonce: u64) -> L1toL2MessagingEventData {
        L1toL2MessagingEventData {
            block_number,
            transaction_hash: Felt::from(1),
            event_index: Some(0),
            from: Felt::from(123),
            to: Felt::from(456),
            selector: Felt::from(789),
            payload: vec![Felt::from(1), Felt::from(2)],
            nonce: Felt::from(nonce),
            fee: Some(1000),
            message_hash: None,
        }
    }

    struct MessagingTestRunner {
        client: MockSettlementClientTrait,
        db: Arc<DatabaseService>,
        mempool: Arc<Mempool>,
        ctx: ServiceContext,
    }

    #[fixture]
    async fn setup_messaging_tests() -> MessagingTestRunner {
        // Set up chain info
        let chain_config = Arc::new(ChainConfig::madara_test());

        // Initialize database service
        let db = Arc::new(DatabaseService::open_for_testing(chain_config.clone()));

        let l1_gas_setter = GasPriceProvider::new();
        let l1_data_provider: Arc<dyn L1DataProvider> = Arc::new(l1_gas_setter.clone());

        let mempool = Arc::new(Mempool::new(
            Arc::clone(db.backend()),
            Arc::clone(&l1_data_provider),
            MempoolLimits::for_testing(),
        ));

        // Create a mock client directly
        let mut mock_client = MockSettlementClientTrait::default();

        // Configure basic mock expectations that all tests will need
        mock_client.expect_get_client_type().returning(|| ClientType::ETH);

        // Create a new service context for testing
        let ctx = ServiceContext::new_for_testing();

        MessagingTestRunner { client: mock_client, db, mempool, ctx }
    }

    #[rstest]
    #[tokio::test]
    async fn test_sync_processes_new_message(
        #[future] setup_messaging_tests: MessagingTestRunner,
    ) -> anyhow::Result<()> {
        let MessagingTestRunner { mut client, db, mempool, ctx } = setup_messaging_tests.await;

        // Setup mock event
        let mock_event = create_mock_event(100, 1);
        let event_clone = mock_event.clone();

        let backend = db.backend();

        // Setup mock for last synced block
        backend.messaging_update_last_synced_l1_block_with_event(LastSyncedEventBlock::new(99, 0))?;

        // Mock get_messaging_stream
        client
            .expect_get_messaging_stream()
            .times(1)
            .returning(|_| Ok(Box::pin(stream::once(async move { Ok(create_mock_event(100, 1)) }))));

        // Mock get_messaging_hash
        client.expect_get_messaging_hash().times(1).returning(|_| Ok(vec![0u8; 32]));

        // Mock get_l1_to_l2_message_cancellations
        client.expect_get_l1_to_l2_message_cancellations().times(1).returning(|_| Ok(Felt::ZERO));

        let client: Arc<dyn SettlementClientTrait<Config = DummyConfig, StreamType = DummyStream>> = Arc::new(client);

        timeout(Duration::from_secs(1), sync(client, backend.clone(), mempool.clone(), ctx)).await??;

        // Verify the message was processed
        assert!(backend.has_l1_messaging_nonce(Nonce(event_clone.nonce))?);

        Ok(())
    }

    #[rstest]
    #[tokio::test]
    async fn test_sync_handles_cancelled_message(
        #[future] setup_messaging_tests: MessagingTestRunner,
    ) -> anyhow::Result<()> {
        let MessagingTestRunner { mut client, db, mempool, ctx } = setup_messaging_tests.await;

        let backend = db.backend();

        // Setup mock event
        let mock_event = create_mock_event(100, 1);
        let event_clone = mock_event.clone();

        // Setup mock for last synced block
        backend.messaging_update_last_synced_l1_block_with_event(LastSyncedEventBlock::new(99, 0))?;

        // Mock get_messaging_stream
        client
            .expect_get_messaging_stream()
            .times(1)
            .returning(move |_| Ok(Box::pin(stream::iter(vec![Ok(mock_event.clone())]))));

        // Mock get_messaging_hash
        client.expect_get_messaging_hash().times(1).returning(|_| Ok(vec![0u8; 32]));

        // Mock get_l1_to_l2_message_cancellations - return non-zero to indicate cancellation
        client.expect_get_l1_to_l2_message_cancellations().times(1).returning(|_| Ok(Felt::from(12345)));

        let client: Arc<dyn SettlementClientTrait<Config = DummyConfig, StreamType = DummyStream>> = Arc::new(client);

        timeout(Duration::from_secs(1), sync(client, backend.clone(), mempool.clone(), ctx)).await??;

        // Verify the cancelled message was handled correctly
        assert!(backend.has_l1_messaging_nonce(Nonce(event_clone.nonce))?);

        Ok(())
    }

    #[rstest]
    #[tokio::test]
    async fn test_sync_skips_already_processed_message(
        #[future] setup_messaging_tests: MessagingTestRunner,
    ) -> anyhow::Result<()> {
        let MessagingTestRunner { mut client, db, mempool, ctx } = setup_messaging_tests.await;

        let backend = db.backend();

        // Setup mock event
        let mock_event = create_mock_event(100, 1);

        // Pre-set the nonce as processed
        backend.set_l1_messaging_nonce(Nonce(mock_event.nonce))?;

        // Setup mock for last synced block
        backend.messaging_update_last_synced_l1_block_with_event(LastSyncedEventBlock::new(99, 0))?;

        // Mock get_messaging_stream
        client
            .expect_get_messaging_stream()
            .times(1)
            .returning(move |_| Ok(Box::pin(stream::iter(vec![Ok(mock_event.clone())]))));

        // Mock get_messaging_hash - should not be called
        client.expect_get_messaging_hash().times(0);

        let client: Arc<dyn SettlementClientTrait<Config = DummyConfig, StreamType = DummyStream>> = Arc::new(client);

        timeout(Duration::from_secs(1), sync(client, backend.clone(), mempool.clone(), ctx)).await??;

        Ok(())
    }

    #[rstest]
    #[tokio::test]
    async fn test_sync_handles_stream_errors(
        #[future] setup_messaging_tests: MessagingTestRunner,
    ) -> anyhow::Result<()> {
        let MessagingTestRunner { mut client, db, mempool, ctx } = setup_messaging_tests.await;

        let backend = db.backend();

        // Setup mock for last synced block
        backend.messaging_update_last_synced_l1_block_with_event(LastSyncedEventBlock::new(99, 0))?;

        // Mock get_messaging_stream to return error
        client.expect_get_messaging_stream().times(1).returning(move |_| {
            Ok(Box::pin(stream::iter(vec![Err(SettlementClientError::Other("Stream error".to_string()))])))
        });

        let client: Arc<dyn SettlementClientTrait<Config = DummyConfig, StreamType = DummyStream>> = Arc::new(client);

        let result = sync(client, backend.clone(), mempool.clone(), ctx).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Stream error"));

        Ok(())
    }
}
