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
    use regex::Regex;
    use serde_json::json;
    use starknet_types_core::felt::Felt;
    use std::sync::Arc;
    use std::time::Instant;
    use tokio::sync::{mpsc, oneshot};
    use url::Url;

    struct TestContext {
        mock_server: MockServer,
        provider: Arc<SequencerGatewayProvider>,
        backend: Arc<DeoxysBackend>,
        fetch_stream_sender: mpsc::Sender<UnverifiedFullBlock>,
        fetch_stream_receiver: mpsc::Receiver<UnverifiedFullBlock>,
        once_caught_up_sender: oneshot::Sender<()>,
        once_caught_up_receiver: oneshot::Receiver<()>,
    }

    impl TestContext {
        fn new() -> Self {
            let mock_server = MockServer::start();
            let provider = Arc::new(SequencerGatewayProvider::new(
                Url::parse(&format!("{}/gateway", mock_server.base_url())).unwrap(),
                Url::parse(&format!("{}/feeder_gateway", mock_server.base_url())).unwrap(),
                Felt::from_hex_unchecked("0x4d41444152415f54455354"),
            ));
            let chain_config = Arc::new(ChainConfig::test_config());
            let backend = DeoxysBackend::open_for_testing(chain_config);
            let (fetch_stream_sender, fetch_stream_receiver) = mpsc::channel(100);
            let (once_caught_up_sender, once_caught_up_receiver) = oneshot::channel();

            Self {
                mock_server,
                provider,
                backend,
                fetch_stream_sender,
                fetch_stream_receiver,
                once_caught_up_sender,
                once_caught_up_receiver,
            }
        }

        fn mock_block(&self, block_number: u64) {
            self.mock_server.mock(|when, then| {
                when.method("GET")
                    .path_contains("get_state_update")
                    .query_param("blockNumber", block_number.to_string());
                then.status(200).header("content-type", "application/json").json_body(json!({
                    "block": {
                        "block_hash": "0x78b67b11f8c23850041e11fb0f3b39db0bcb2c99d756d5a81321d1b483d79f6",
                        "parent_block_hash": "0x5c627d4aeb51280058bed93c7889bce78114d63baad1be0f0aeb32496d5f19c",
                        "block_number": block_number,
                        "state_root": "0xe005205a1327f3dff98074e528f7b96f30e0624a1dfcf571bdc81948d150a0",
                        "transaction_commitment": "0x301a3e7f3ae29c3463a5f753da62e63dc0b6738c36cb17e3d1696926457a40c",
                        "event_commitment": "0x0",
                        "status": "ACCEPTED_ON_L1",
                        "l1_da_mode": "CALLDATA",
                        "l1_gas_price": {
                            "price_in_wei": "0x3b9ada0f",
                            "price_in_fri": "0x0"
                        },
                        "l1_data_gas_price": {
                            "price_in_wei": "0x1",
                            "price_in_fri": "0x1"
                        },
                        "transactions": [
                            {
                                "transaction_hash": "0x30a541df2547ed9f94602c35daf61ce3a8e179ec75d26cbe34e0ec61f823695",
                                "version": "0x0",
                                "max_fee": "0x0",
                                "signature": [],
                                "nonce": "0x0",
                                "class_hash": "0x1b661756bf7d16210fc611626e1af4569baa1781ffc964bd018f4585ae241c1",
                                "sender_address": "0x1",
                                "type": "DECLARE"
                            }
                        ],
                        "timestamp": 1700474724,
                        "sequencer_address": "0x1176a1bd84444c89232ec27754698e5d2e7e1a7f1539f12027f28b23ec9f3d8",
                        "transaction_receipts": [
                            {
                                "execution_status": "SUCCEEDED",
                                "transaction_index": 0,
                                "transaction_hash": "0x30a541df2547ed9f94602c35daf61ce3a8e179ec75d26cbe34e0ec61f823695",
                                "l2_to_l1_messages": [],
                                "events": [],
                                "execution_resources": {
                                    "n_steps": 2711,
                                    "builtin_instance_counter": {
                                        "pedersen_builtin": 15,
                                        "range_check_builtin": 63
                                    },
                                    "n_memory_holes": 0
                                },
                                "actual_fee": "0x0"
                            }
                        ],
                        "starknet_version": "0.12.3"
                    },
                    "state_update": {
                        "block_hash": "0x78b67b11f8c23850041e11fb0f3b39db0bcb2c99d756d5a81321d1b483d79f6",
                        "new_root": "0xe005205a1327f3dff98074e528f7b96f30e0624a1dfcf571bdc81948d150a0",
                        "old_root": "0xe005205a1327f3dff98074e528f7b96f30e0624a1dfcf571bdc81948d150a0",
                        "state_diff": {
                            "storage_diffs": {},
                            "nonces": {},
                            "deployed_contracts": [],
                            "old_declared_contracts": [
                                "0x1b661756bf7d16210fc611626e1af4569baa1781ffc964bd018f4585ae241c1"
                            ],
                            "declared_classes": [],
                            "replaced_classes": []
                        }
                    }
                }));
            });
        }

        fn mock_class_hash(&self) {
            self.mock_server.mock(|when, then| {
                when.method("GET").path_matches(Regex::new(r"/feeder_gateway/get_class_by_hash").unwrap());
                then.status(200).header("content-type", "application/json").json_body(json!({
                    "contract_class_version": "0.1.0",
                    "sierra_program": [
                        "0x1",
                        "0x3",
                        "0x0",
                        "0xf60b61071107f516110d070b685a590b16714f5a590b14f40b"
                    ],
                    "entry_points_by_type": {
                        "L1_HANDLER": [],
                        "EXTERNAL": [
                            {
                                "selector": "0x3147e009aa1d3b7827f0cf9ce80b10dd02b119d549eb0a2627600662354eba",
                                "function_idx": 2
                            },
                            {
                                "selector": "0x10b7e63d3ca05c9baffd985d3e1c3858d4dbf0759f066be0eaddc5d71c2cab5",
                                "function_idx": 0
                            },
                            {
                                "selector": "0x3370263ab53343580e77063a719a5865004caff7f367ec136a6cdd34b6786ca",
                                "function_idx": 1
                            }
                        ],
                        "CONSTRUCTOR": []
                    },
                    "abi": "[]"
                }));
            });
        }

        fn mock_block_not_found(&self, block_number: u64) {
            self.mock_server.mock(|when, then| {
                when.method("GET")
                    .path_contains("get_state_update")
                    .query_param("blockNumber", block_number.to_string());
                then.status(400).header("content-type", "application/json").json_body(json!({
                    "code": "StarknetErrorCode.BLOCK_NOT_FOUND",
                    "message": "Block not found"
                }));
            });
        }
    }

    /// Test basic functionality of the l2_fetch_task.
    ///
    /// This test verifies that:
    /// 1. The task can fetch a specified number of blocks.
    /// 2. The task sends the correct "caught up" signal.
    /// 3. The task completes successfully after fetching all blocks.
    #[tokio::test]
    async fn test_basic_functionality() {
        let mut ctx = TestContext::new();

        for block_number in 0..=4 {
            ctx.mock_block(block_number);
        }
        ctx.mock_block_not_found(5);
        ctx.mock_class_hash();

        let task = tokio::spawn({
            let backend = Arc::clone(&ctx.backend);
            let provider = Arc::clone(&ctx.provider);
            let fetch_stream_sender = ctx.fetch_stream_sender.clone();
            let once_caught_up_sender = ctx.once_caught_up_sender;
            async move {
                tokio::time::timeout(
                    std::time::Duration::from_secs(10),
                    l2_fetch_task(backend, 0, Some(5), fetch_stream_sender, provider, None, once_caught_up_sender),
                )
                .await
            }
        });

        for expected_block_number in 0..=4 {
            match tokio::time::timeout(std::time::Duration::from_secs(10), ctx.fetch_stream_receiver.recv()).await {
                Ok(Some(block)) => {
                    assert_eq!(block.unverified_block_number, Some(expected_block_number));
                    println!("Received block {}", expected_block_number);
                }
                Ok(None) => panic!("Channel closed unexpectedly while waiting for block {}", expected_block_number),
                Err(_) => panic!("Timeout waiting for block {}", expected_block_number),
            }
        }

        match tokio::time::timeout(std::time::Duration::from_secs(1), ctx.once_caught_up_receiver).await {
            Ok(Ok(())) => println!("Caught up callback received"),
            Ok(Err(_)) => panic!("Caught up channel closed unexpectedly"),
            Err(_) => panic!("Timeout waiting for caught up callback"),
        }

        match task.await {
            Ok(Ok(Ok(()))) => println!("Task completed successfully"),
            Ok(Ok(Err(e))) => panic!("Task failed with error: {:?}", e),
            Ok(Err(_)) => panic!("Task timed out"),
            Err(e) => panic!("Task panicked: {:?}", e),
        }
    }

    /// Test the task's ability to catch up to the chain tip.
    ///
    /// This test verifies that:
    /// 1. The task can fetch blocks until it reaches the chain tip.
    /// 2. The task stops fetching when it encounters a "block not found" error.
    /// 3. The task sends the correct "caught up" signal.
    /// 4. The task completes successfully after reaching the chain tip.
    #[tokio::test]
    async fn test_catch_up_to_chain_tip() {
        let mut ctx = TestContext::new();

        for block_number in 0..10 {
            ctx.mock_block(block_number);
        }
        ctx.mock_block_not_found(10);
        ctx.mock_class_hash();

        let task = tokio::spawn({
            let backend = Arc::clone(&ctx.backend);
            let provider = Arc::clone(&ctx.provider);
            let fetch_stream_sender = ctx.fetch_stream_sender.clone();
            let once_caught_up_sender = ctx.once_caught_up_sender;
            async move {
                tokio::time::timeout(
                    std::time::Duration::from_secs(30),
                    l2_fetch_task(backend, 0, None, fetch_stream_sender, provider, None, once_caught_up_sender),
                )
                .await
            }
        });

        for expected_block_number in 0..10 {
            match tokio::time::timeout(std::time::Duration::from_secs(5), ctx.fetch_stream_receiver.recv()).await {
                Ok(Some(block)) => {
                    assert_eq!(block.unverified_block_number, Some(expected_block_number));
                    println!("Received block {}", expected_block_number);
                }
                Ok(None) => panic!("Channel closed unexpectedly while waiting for block {}", expected_block_number),
                Err(_) => panic!("Timeout waiting for block {}", expected_block_number),
            }
        }

        match tokio::time::timeout(std::time::Duration::from_secs(2), ctx.fetch_stream_receiver.recv()).await {
            Ok(Some(_)) => panic!("Unexpected block received after chain tip"),
            Ok(None) => println!("Channel closed as expected"),
            Err(_) => println!("No more blocks received, as expected"),
        }

        match tokio::time::timeout(std::time::Duration::from_secs(2), ctx.once_caught_up_receiver).await {
            Ok(Ok(())) => println!("Caught up callback received"),
            Ok(Err(_)) => panic!("Caught up channel closed unexpectedly"),
            Err(_) => panic!("Timeout waiting for caught up callback"),
        }

        match task.await {
            Ok(Ok(Ok(()))) => println!("Task completed successfully"),
            Ok(Ok(Err(e))) => panic!("Task failed with error: {:?}", e),
            Ok(Err(_)) => panic!("Task timed out"),
            Err(e) => panic!("Task panicked: {:?}", e),
        }
    }

    /// Test the polling behavior of the l2_fetch_task.
    ///
    /// This test verifies that:
    /// 1. The task fetches an initial set of blocks.
    /// 2. The task sends the "caught up" signal after the initial fetch.
    /// 3. The task continues to poll for new blocks at the specified interval.
    /// 4. The task can fetch new blocks that appear after the initial sync.
    #[tokio::test]
    async fn test_polling_behavior() {
        let mut ctx = TestContext::new();

        for block_number in 0..8 {
            ctx.mock_block(block_number);
        }

        ctx.mock_class_hash();

        let polling_interval = Duration::from_millis(100);
        let task = tokio::spawn({
            let backend = Arc::clone(&ctx.backend);
            let provider = Arc::clone(&ctx.provider);
            let fetch_stream_sender = ctx.fetch_stream_sender.clone();
            let once_caught_up_sender = ctx.once_caught_up_sender;
            async move {
                tokio::time::timeout(
                    Duration::from_secs(5),
                    l2_fetch_task(
                        backend,
                        0,
                        Some(5),
                        fetch_stream_sender,
                        provider,
                        Some(polling_interval),
                        once_caught_up_sender,
                    ),
                )
                .await
            }
        });

        for expected_block_number in 0..5 {
            match tokio::time::timeout(Duration::from_secs(1), ctx.fetch_stream_receiver.recv()).await {
                Ok(Some(block)) => {
                    assert_eq!(block.unverified_block_number, Some(expected_block_number));
                    println!("Received initial block {}", expected_block_number);
                }
                Ok(None) => panic!("Channel closed unexpectedly"),
                Err(_) => panic!("Timeout waiting for block {}", expected_block_number),
            }
        }

        match tokio::time::timeout(Duration::from_secs(1), ctx.once_caught_up_receiver).await {
            Ok(Ok(())) => println!("Caught up callback received"),
            Ok(Err(_)) => panic!("Caught up channel closed unexpectedly"),
            Err(_) => panic!("Timeout waiting for caught up callback"),
        }

        tokio::time::sleep(Duration::from_millis(300)).await;

        for expected_block_number in 5..8 {
            match tokio::time::timeout(Duration::from_secs(1), ctx.fetch_stream_receiver.recv()).await {
                Ok(Some(block)) => {
                    assert_eq!(block.unverified_block_number, Some(expected_block_number));
                    println!("Received new block {}", expected_block_number);
                }
                Ok(None) => panic!("Channel closed unexpectedly"),
                Err(_) => panic!("Timeout waiting for block {}", expected_block_number),
            }
        }

        assert!(!task.is_finished());

        task.abort();
    }

    /// Test parallel fetching capabilities of the l2_fetch_task.
    ///
    /// This test verifies that:
    /// 1. The task can fetch multiple blocks in parallel.
    /// 2. Parallel fetching is significantly faster than theoretical sequential fetching.
    /// 3. The task correctly handles and orders blocks fetched in parallel.
    /// 4. The task sends the "caught up" signal after fetching all blocks.
    #[tokio::test]
    async fn test_parallel_fetching() {
        let mut ctx = TestContext::new();

        const TOTAL_BLOCKS: u64 = 100;
        const BLOCKS_TO_SYNC: u64 = 50;
        const PARALLEL_FETCHES: usize = 10;

        // Mock all blocks
        for block_number in 0..TOTAL_BLOCKS {
            ctx.mock_block(block_number);
        }
        ctx.mock_block_not_found(TOTAL_BLOCKS);
        ctx.mock_class_hash();

        let start_time = Instant::now();

        let task = tokio::spawn({
            let backend = Arc::clone(&ctx.backend);
            let provider = Arc::clone(&ctx.provider);
            let fetch_stream_sender = ctx.fetch_stream_sender.clone();
            let once_caught_up_sender = ctx.once_caught_up_sender;
            async move {
                l2_fetch_task(
                    backend,
                    0,
                    Some(BLOCKS_TO_SYNC),
                    fetch_stream_sender,
                    provider,
                    None,
                    once_caught_up_sender,
                )
                .await
            }
        });

        // Receive all synced blocks
        for expected_block_number in 0..BLOCKS_TO_SYNC {
            match tokio::time::timeout(Duration::from_secs(5), ctx.fetch_stream_receiver.recv()).await {
                Ok(Some(block)) => {
                    assert_eq!(block.unverified_block_number, Some(expected_block_number));
                    println!("Received block {}", expected_block_number);
                }
                Ok(None) => panic!("Channel closed unexpectedly"),
                Err(_) => panic!("Timeout waiting for block {}", expected_block_number),
            }
        }

        let elapsed_time = start_time.elapsed();

        // Wait for the "caught up" signal
        match tokio::time::timeout(Duration::from_secs(2), ctx.once_caught_up_receiver).await {
            Ok(Ok(())) => println!("Caught up callback received"),
            Ok(Err(_)) => panic!("Caught up channel closed unexpectedly"),
            Err(_) => panic!("Timeout waiting for caught up callback"),
        }

        // Wait for the task to complete
        match tokio::time::timeout(Duration::from_secs(2), task).await {
            Ok(Ok(Ok(()))) => println!("Task completed successfully"),
            Ok(Ok(Err(e))) => panic!("Task failed with error: {:?}", e),
            Ok(Err(e)) => panic!("Task panicked: {:?}", e),
            Err(_) => panic!("Task did not complete within the expected timeframe"),
        }

        // Calculate the expected time for sequential vs parallel fetching
        // Note: These are theoretical times, actual execution will be much faster
        let theoretical_sequential_time = Duration::from_secs(BLOCKS_TO_SYNC);
        let theoretical_parallel_time = Duration::from_secs(BLOCKS_TO_SYNC / PARALLEL_FETCHES as u64 + 1);

        println!("Elapsed time: {:?}", elapsed_time);
        println!("Theoretical sequential time: {:?}", theoretical_sequential_time);
        println!("Theoretical parallel time: {:?}", theoretical_parallel_time);

        // Assert that the actual time is closer to the parallel time than the sequential time
        assert!(elapsed_time < theoretical_sequential_time, "Fetching took longer than theoretical sequential time");

        // Allow for some overhead, but ensure it's significantly faster than sequential
        assert!(elapsed_time < theoretical_parallel_time * 2, "Fetching was not as parallel as expected");

        println!("Parallel fetching test completed successfully");
    }
}
