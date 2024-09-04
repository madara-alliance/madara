use std::sync::Arc;
use std::time::Duration;

use futures::prelude::*;
use mc_block_import::UnverifiedFullBlock;
use mc_db::MadaraBackend;
use mp_utils::{channel_wait_or_graceful_shutdown, wait_or_graceful_shutdown};
use starknet_core::types::StarknetError;
use starknet_providers::{ProviderError, SequencerGatewayProvider};
use tokio::sync::{mpsc, oneshot};

use crate::fetch::fetchers::fetch_block_and_updates;

pub mod fetchers;

#[allow(clippy::too_many_arguments)]
pub async fn l2_fetch_task(
    backend: Arc<MadaraBackend>,
    first_block: u64,
    n_blocks_to_sync: Option<u64>,
    fetch_stream_sender: mpsc::Sender<UnverifiedFullBlock>,
    provider: Arc<SequencerGatewayProvider>,
    sync_polling_interval: Option<Duration>,
    once_caught_up_callback: oneshot::Sender<()>,
) -> anyhow::Result<()> {
    // First, catch up with the chain
    let backend = &backend;

    let mut next_block = first_block;

    {
        // Fetch blocks and updates in parallel one time before looping
        let fetch_stream = (first_block..).take(n_blocks_to_sync.unwrap_or(u64::MAX) as _).map(|block_n| {
            let provider = Arc::clone(&provider);
            async move { (block_n, fetch_block_and_updates(backend, block_n, &provider).await) }
        });

        // Have 10 fetches in parallel at once, using futures Buffered
        let mut fetch_stream = stream::iter(fetch_stream).buffered(10);
        while let Some((block_n, val)) = channel_wait_or_graceful_shutdown(fetch_stream.next()).await {
            log::debug!("got {:?}", block_n);

            match val {
                Err(FetchError::Provider(ProviderError::StarknetError(StarknetError::BlockNotFound))) => {
                    log::info!("🥳 The sync process has caught up with the tip of the chain");
                    break;
                }
                val => {
                    if fetch_stream_sender.send(val?).await.is_err() {
                        // join error
                        break;
                    }
                }
            }

            next_block = block_n + 1;
        }
    };

    log::debug!("caught up with tip");
    let _ = once_caught_up_callback.send(());

    if let Some(sync_polling_interval) = sync_polling_interval {
        // Polling

        let mut interval = tokio::time::interval(sync_polling_interval);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        while wait_or_graceful_shutdown(interval.tick()).await.is_some() {
            loop {
                match fetch_block_and_updates(backend, next_block, &provider).await {
                    Err(FetchError::Provider(ProviderError::StarknetError(StarknetError::BlockNotFound))) => {
                        break;
                    }
                    val => {
                        if fetch_stream_sender.send(val?).await.is_err() {
                            // stream closed
                            break;
                        }
                    }
                }

                next_block += 1;
            }
        }
    }
    Ok(())
}

#[derive(thiserror::Error, Debug)]
pub enum FetchError {
    #[error(transparent)]
    Provider(#[from] ProviderError),
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

#[cfg(test)]
mod test_l2_fetch_task {
    use super::*;
    use dp_chain_config::ChainConfig;
    use httpmock::MockServer;
    use starknet_types_core::felt::Felt;
    use std::sync::Arc;
    use tokio::sync::{mpsc, oneshot};
    use url::Url;

    struct TestMocks {
        backend: Arc<DeoxysBackend>,
        provider: Arc<SequencerGatewayProvider>,
        fetch_stream_sender: mpsc::Sender<UnverifiedFullBlock>,
        fetch_stream_receiver: mpsc::Receiver<UnverifiedFullBlock>,
        once_caught_up_sender: oneshot::Sender<()>,
        once_caught_up_receiver: oneshot::Receiver<()>,
        mock_server: MockServer,
    }

    fn setup_mocks() -> TestMocks {
        // Setup mock server
        let mock_server = MockServer::start();

        // Create SequencerGatewayProvider with mock server URL
        // Create SequencerGatewayProvider with mock server URL
        let provider = Arc::new(SequencerGatewayProvider::new(
            Url::parse(&format!("{}/gateway", mock_server.base_url())).unwrap(),
            Url::parse(&format!("{}/feeder_gateway", mock_server.base_url())).unwrap(),
            Felt::from_hex_unchecked("0x4d41444152415f54455354"), // Dummy chain ID
        ));

        // Initialize database service
        let chain_config = Arc::new(ChainConfig::test_config());
        let backend = DeoxysBackend::open_for_testing(chain_config.clone());

        // Create channels
        let (fetch_stream_sender, fetch_stream_receiver) = mpsc::channel(100);
        let (once_caught_up_sender, once_caught_up_receiver) = oneshot::channel();

        TestMocks {
            backend,
            provider,
            fetch_stream_sender,
            fetch_stream_receiver,
            once_caught_up_sender,
            once_caught_up_receiver,
            mock_server,
        }
    }

    #[tokio::test]
    async fn test_basic_functionality() {
        let TestMocks {
            backend,
            provider,
            fetch_stream_sender,
            mut fetch_stream_receiver,
            once_caught_up_sender,
            once_caught_up_receiver,
            mock_server,
        } = setup_mocks();

        // Mock server to return 5 state updates with blocks
        for i in 0..5 {
            let block_number = i;
            mock_server.mock(|when, then| {
                when.method("GET").path(format!("/feeder_gateway/get_state_update?blockNumber={}", block_number));
                then.status(200).header("content-type", "application/json").body(format!(
                    r#"{{
                        "block_hash": "0x{}",
                        "new_root": "0x123456",
                        "old_root": "0x654321",
                        "state_diff": {{
                            "storage_diffs": [],
                            "deployed_contracts": [
                                {{"address": "0x1234", "class_hash": "0x5678"}}
                            ],
                            "declared_classes": [
                                {{"class_hash": "0x9abc", "compiled_class_hash": "0xdef0"}}
                            ],
                            "old_declared_contracts": ["0xfedc"]
                        }},
                        "block": {{
                            "block_number": {},
                            "state_root": "0x123456",
                            "transactions": [],
                            "timestamp": 1000000,
                            "sequencer_address": "0x1234567890",
                            "parent_block_hash": "0x0987654321",
                            "status": "ACCEPTED_ON_L2",
                            "gas_price": {{"price_in_wei": "0x1", "price_in_fri": "0x1"}},
                            "starknet_version": "0.11.0",
                            "l1_gas_price": {{"price_in_wei": "0x1", "price_in_fri": "0x1"}},
                            "l1_data_gas_price": {{"price_in_wei": "0x1", "price_in_fri": "0x1"}},
                            "l1_da_mode": "CALLDATA"
                        }}
                    }}"#,
                    block_number.to_string().repeat(64),
                    block_number
                ));
            });

            // Mock get_class endpoint for each class hash in the state update
            for class_hash in ["0x5678", "0x9abc", "0xfedc"] {
                mock_server.mock(|when, then| {
                    when.method("GET").path(format!(
                        "/feeder_gateway/get_class_by_hash?classHash={}&blockNumber={}",
                        class_hash, block_number
                    ));
                    then.status(200).header("content-type", "application/json").body(
                        r#"{
                            "abi": [],
                            "entry_points_by_type": {
                                "CONSTRUCTOR": [],
                                "EXTERNAL": [],
                                "L1_HANDLER": []
                            },
                            "sierra_program": ["0x1", "0x2", "0x3"],
                            "contract_class_version": "0.1.0"
                        }"#,
                    );
                });
            }
        }

        // Mock server to return BlockNotFound for the 6th request
        mock_server.mock(|when, then| {
            when.method("GET").path("/feeder_gateway/get_state_update?blockNumber=5");
            then.status(404)
                .header("content-type", "application/json")
                .body(r#"{"code": "StarknetErrorCode.BLOCK_NOT_FOUND", "message": "Block not found"}"#);
        });

        // Call l2_fetch_task with a timeout
        let task = tokio::spawn(async move {
            tokio::time::timeout(
                std::time::Duration::from_secs(10),
                l2_fetch_task(backend, 0, Some(5), fetch_stream_sender, provider, None, once_caught_up_sender),
            )
            .await
        });

        // Assert that 5 blocks were received
        for i in 0..5 {
            match tokio::time::timeout(std::time::Duration::from_secs(5), fetch_stream_receiver.recv()).await {
                Ok(Some(block)) => {
                    assert_eq!(block.unverified_block_number, Some(i as u64));
                    println!("Received block {}", i);
                }
                Ok(None) => panic!("Channel closed unexpectedly"),
                Err(_) => panic!("Timeout waiting for block {}", i),
            }
        }

        // Assert that once_caught_up_callback was triggered
        match tokio::time::timeout(std::time::Duration::from_secs(1), once_caught_up_receiver).await {
            Ok(Ok(())) => println!("Caught up callback received"),
            Ok(Err(_)) => panic!("Caught up channel closed unexpectedly"),
            Err(_) => panic!("Timeout waiting for caught up callback"),
        }

        // Ensure the task completed successfully
        match task.await {
            Ok(Ok(Ok(()))) => println!("Task completed successfully"),
            Ok(Ok(Err(e))) => panic!("Task failed with error: {:?}", e),
            Ok(Err(_)) => panic!("Task timed out"),
            Err(e) => panic!("Task panicked: {:?}", e),
        }
    }

    #[tokio::test]
    async fn test_catch_up_to_chain_tip() {
        // TODO: Implement test for catching up to chain tip
        // - Mock server to return BlockNotFound after X blocks
        // - Call l2_fetch_task with n_blocks_to_sync set to None
        // - Assert that the function stops after receiving BlockNotFound
        // - Assert that once_caught_up_callback was triggered
    }

    #[tokio::test]
    async fn test_polling_behavior() {
        // TODO: Implement test for polling behavior
        // - Mock server to return BlockNotFound after X blocks, then return new blocks
        // - Call l2_fetch_task with a small sync_polling_interval
        // - Assert that function continues to fetch new blocks after initial catch-up
        // - Assert that once_caught_up_callback was triggered only once
    }

    #[tokio::test]
    async fn test_error_handling() {
        // TODO: Implement test for error handling
        // - Mock server to occasionally return errors
        // - Call l2_fetch_task and observe its behavior
        // - Assert that it correctly handles and potentially retries on errors
        // - Assert that it doesn't crash on recoverable errors
    }

    #[tokio::test]
    async fn test_graceful_shutdown() {
        // TODO: Implement test for graceful shutdown
        // - Start l2_fetch_task in a separate task
        // - Trigger a graceful shutdown signal
        // - Assert that the function terminates cleanly
    }

    #[tokio::test]
    async fn test_large_number_of_blocks() {
        // TODO: Implement test for fetching a large number of blocks
        // - Mock server to return many blocks
        // - Call l2_fetch_task with n_blocks_to_sync set to a large number (e.g., 10000)
        // - Assert that all blocks are fetched correctly
        // - Monitor memory usage and performance
    }

    #[tokio::test]
    async fn test_parallel_fetching() {
        // TODO: Implement test for parallel fetching
        // - Mock server to add a delay to each block fetch
        // - Call l2_fetch_task with n_blocks_to_sync set to a moderate number
        // - Measure the total time taken
        // - Assert that the time taken is significantly less than sequential fetching
    }

    #[tokio::test]
    async fn test_channel_backpressure() {
        // TODO: Implement test for channel backpressure
        // - Create a slow receiver for fetch_stream_sender
        // - Call l2_fetch_task with a fast mock server
        // - Assert that the function handles backpressure correctly
    }

    #[tokio::test]
    async fn test_provider_errors() {
        // TODO: Implement test for various provider errors
        // - Mock server to return different types of errors
        // - Call l2_fetch_task multiple times with different error scenarios
        // - Assert that each error type is handled appropriately
    }

    #[tokio::test]
    async fn test_resume_from_specific_block() {
        // TODO: Implement test for resuming from a specific block
        // - Call l2_fetch_task with first_block set to a non-zero value
        // - Assert that fetching starts from the specified block
    }
}
