use crate::core::config::StarknetVersion;
use crate::tests::config::{ConfigType, TestConfigBuilder};
use crate::types::batch::{SnosBatch, SnosBatchStatus};
use crate::worker::event_handler::triggers::snos_batching::SnosBatchingTrigger;
use crate::worker::event_handler::triggers::JobTrigger;
use httpmock::MockServer;
use orchestrator_utils::layer::Layer;
use rstest::rstest;
use serde_json::json;
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::JsonRpcClient;
use std::error::Error;
use url::Url;

/// Test: SNOS batching works independently in L3 mode
///
/// This test validates that when operating in L3 mode:
/// - SNOS batching does NOT depend on aggregator batches
/// - Batches are created with aggregator_batch_index = None
/// - Range calculation is based on RPC block_number, not aggregator batches
#[rstest]
#[tokio::test]
async fn test_snos_l3_independent_batching() -> Result<(), Box<dyn Error>> {
    let server = MockServer::start();
    let provider_url = format!("http://localhost:{}", server.port());

    // Setup: No aggregator batches needed for L3
    let provider = JsonRpcClient::new(HttpTransport::new(Url::parse(&provider_url).expect("Failed to parse URL")));

    let services = TestConfigBuilder::new()
        .configure_starknet_client(provider.into())
        .configure_storage_client(ConfigType::Actual)
        .configure_database(ConfigType::Actual)
        .configure_lock_client(ConfigType::Actual)
        .configure_layer(Layer::L3) // L3 mode
        .configure_madara_feeder_gateway_url(&provider_url)
        .configure_max_blocks_per_snos_batch(Some(5)) // Limit batch size for testing
        .build()
        .await;

    let database = services.config.database();

    // Setup: Blocks 0-9 available from RPC
    let start_block = 0;
    let end_block = 9;

    // Mock block number call
    server.mock(|when, then| {
        when.path("/").body_includes("starknet_blockNumber");
        then.status(200)
            .body(serde_json::to_vec(&json!({ "id": 1, "jsonrpc": "2.0", "result": end_block })).unwrap());
    });

    // Mock bouncer weights calls
    let builtin_weights = crate::tests::workers::batching::helpers::generate_bouncer_weights();
    server.mock(|when, then| {
        when.method(httpmock::Method::GET).path("/feeder_gateway/get_block_bouncer_weights");
        then.status(200)
            .header("Content-Type", "application/json")
            .body(serde_json::to_vec(&builtin_weights).unwrap());
    });

    // Mock getBlockWithTxHashes for version fetching
    for block_num in start_block..=end_block {
        let pattern = format!(r#".*"block_number"\s*:\s*{}[,\}}].*"#, block_num);
        server.mock(move |when, then| {
            when.path("/")
                .body_includes("starknet_getBlockWithTxHashes")
                .body_matches(pattern.as_str());
            then.status(200).body(
                serde_json::to_vec(&json!({
                    "jsonrpc": "2.0",
                    "result": crate::tests::workers::batching::helpers::generate_block_with_tx_hashes(block_num, "0.13.2"),
                    "id": 1
                }))
                .unwrap(),
            );
        });
    }

    // Execute: Run the SNOS batching trigger in L3 mode
    SnosBatchingTrigger.run_worker(services.config.clone()).await?;

    // Assert: Verify SNOS batches were created
    let snos_batches_closed = database.get_snos_batches_without_jobs(SnosBatchStatus::Closed).await?;
    let snos_batches_open = database.get_snos_batches_without_jobs(SnosBatchStatus::Open).await?;

    let all_batches: Vec<SnosBatch> = snos_batches_closed.into_iter().chain(snos_batches_open).collect();

    assert!(!all_batches.is_empty(), "Expected at least one SNOS batch to be created in L3 mode");

    // Assert: All batches should have aggregator_batch_index = None (L3 mode)
    for batch in &all_batches {
        assert_eq!(
            batch.aggregator_batch_index, None,
            "L3 SNOS batch {} should have aggregator_batch_index = None, but got {:?}",
            batch.index, batch.aggregator_batch_index
        );
    }

    // Assert: Batches should cover the available block range
    let first_batch = all_batches.first().unwrap();
    assert_eq!(first_batch.start_block, 0, "First batch should start at block 0");

    Ok(())
}

/// Test: L3 range calculation based on RPC block_number
///
/// This test validates that in L3 mode:
/// - Range calculation uses RPC block_number, not aggregator batch end
/// - Processes up to 25 blocks per run
/// - Continues from last SNOS batch
#[rstest]
#[tokio::test]
async fn test_snos_l3_range_calculation() -> Result<(), Box<dyn Error>> {
    let server = MockServer::start();
    let provider_url = format!("http://localhost:{}", server.port());

    let provider = JsonRpcClient::new(HttpTransport::new(Url::parse(&provider_url).expect("Failed to parse URL")));

    let services = TestConfigBuilder::new()
        .configure_starknet_client(provider.into())
        .configure_storage_client(ConfigType::Actual)
        .configure_database(ConfigType::Actual)
        .configure_lock_client(ConfigType::Actual)
        .configure_layer(Layer::L3)
        .configure_madara_feeder_gateway_url(&provider_url)
        .configure_max_blocks_per_snos_batch(Some(10))
        .build()
        .await;

    let database = services.config.database();

    // Setup: Existing SNOS batch ending at block 9
    let existing_batch = SnosBatch {
        index: 1,
        aggregator_batch_index: None, // L3 mode
        start_block: 0,
        end_block: 9,
        num_blocks: 10,
        builtin_weights: crate::tests::utils::default_test_bouncer_weights(),
        starknet_version: StarknetVersion::V0_13_2,
        status: SnosBatchStatus::Closed,
        ..Default::default()
    };
    database.create_snos_batch(existing_batch).await?;

    // Setup: RPC reports block_number = 49 (40 blocks available)
    let rpc_block_number = 49;

    // Mock block number call
    server.mock(|when, then| {
        when.path("/").body_includes("starknet_blockNumber");
        then.status(200)
            .body(serde_json::to_vec(&json!({ "id": 1, "jsonrpc": "2.0", "result": rpc_block_number })).unwrap());
    });

    // Mock bouncer weights
    let builtin_weights = crate::tests::workers::batching::helpers::generate_bouncer_weights();
    server.mock(|when, then| {
        when.method(httpmock::Method::GET).path("/feeder_gateway/get_block_bouncer_weights");
        then.status(200)
            .header("Content-Type", "application/json")
            .body(serde_json::to_vec(&builtin_weights).unwrap());
    });

    // Mock blocks 10-34 (25 blocks max per run: 10 + 24)
    for block_num in 10..=34 {
        let pattern = format!(r#".*"block_number"\s*:\s*{}[,\}}].*"#, block_num);
        server.mock(move |when, then| {
            when.path("/")
                .body_includes("starknet_getBlockWithTxHashes")
                .body_matches(pattern.as_str());
            then.status(200).body(
                serde_json::to_vec(&json!({
                    "jsonrpc": "2.0",
                    "result": crate::tests::workers::batching::helpers::generate_block_with_tx_hashes(block_num, "0.13.2"),
                    "id": 1
                }))
                .unwrap(),
            );
        });
    }

    // Execute: Run the SNOS batching trigger
    SnosBatchingTrigger.run_worker(services.config.clone()).await?;

    // Assert: Verify batches were created starting from block 10
    let snos_batches_closed = database.get_snos_batches_without_jobs(SnosBatchStatus::Closed).await?;
    let snos_batches_open = database.get_snos_batches_without_jobs(SnosBatchStatus::Open).await?;
    let all_batches: Vec<SnosBatch> = snos_batches_closed.into_iter().chain(snos_batches_open).collect();

    // Filter batches created in this run (start_block >= 10)
    let new_batches: Vec<_> = all_batches.iter().filter(|b| b.start_block >= 10).collect();

    assert!(!new_batches.is_empty(), "Expected new batches to be created starting from block 10");

    let first_new_batch = new_batches.first().unwrap();
    assert_eq!(first_new_batch.start_block, 10, "First new batch should start at block 10");

    // Assert: Should process at most 25 blocks (10-34)
    let last_new_batch = new_batches.last().unwrap();
    assert!(
        last_new_batch.end_block <= 34,
        "Should process at most 25 blocks per run, last batch ends at {}",
        last_new_batch.end_block
    );

    Ok(())
}

/// Test: L3 mode has no aggregator constraint
///
/// This test validates that in L3 mode:
/// - SNOS batches can span any range
/// - No aggregator boundary checking
/// - Batches only close based on size/weight/time/version constraints
#[rstest]
#[tokio::test]
async fn test_snos_l3_no_aggregator_constraint() -> Result<(), Box<dyn Error>> {
    let server = MockServer::start();
    let provider_url = format!("http://localhost:{}", server.port());

    let provider = JsonRpcClient::new(HttpTransport::new(Url::parse(&provider_url).expect("Failed to parse URL")));

    let services = TestConfigBuilder::new()
        .configure_starknet_client(provider.into())
        .configure_storage_client(ConfigType::Actual)
        .configure_database(ConfigType::Actual)
        .configure_lock_client(ConfigType::Actual)
        .configure_layer(Layer::L3)
        .configure_madara_feeder_gateway_url(&provider_url)
        .configure_max_blocks_per_snos_batch(Some(15)) // Larger than typical aggregator batch
        .build()
        .await;

    let database = services.config.database();

    // Setup: Blocks 0-14 available
    let end_block = 14;

    // Mock block number call
    server.mock(|when, then| {
        when.path("/").body_includes("starknet_blockNumber");
        then.status(200)
            .body(serde_json::to_vec(&json!({ "id": 1, "jsonrpc": "2.0", "result": end_block })).unwrap());
    });

    // Mock bouncer weights
    let builtin_weights = crate::tests::workers::batching::helpers::generate_bouncer_weights();
    server.mock(|when, then| {
        when.method(httpmock::Method::GET).path("/feeder_gateway/get_block_bouncer_weights");
        then.status(200)
            .header("Content-Type", "application/json")
            .body(serde_json::to_vec(&builtin_weights).unwrap());
    });

    // Mock blocks
    for block_num in 0..=end_block {
        let pattern = format!(r#".*"block_number"\s*:\s*{}[,\}}].*"#, block_num);
        server.mock(move |when, then| {
            when.path("/")
                .body_includes("starknet_getBlockWithTxHashes")
                .body_matches(pattern.as_str());
            then.status(200).body(
                serde_json::to_vec(&json!({
                    "jsonrpc": "2.0",
                    "result": crate::tests::workers::batching::helpers::generate_block_with_tx_hashes(block_num, "0.13.2"),
                    "id": 1
                }))
                .unwrap(),
            );
        });
    }

    // Execute: Run the SNOS batching trigger
    SnosBatchingTrigger.run_worker(services.config.clone()).await?;

    // Assert: Verify batches were created
    let snos_batches_closed = database.get_snos_batches_without_jobs(SnosBatchStatus::Closed).await?;
    let snos_batches_open = database.get_snos_batches_without_jobs(SnosBatchStatus::Open).await?;
    let all_batches: Vec<SnosBatch> = snos_batches_closed.into_iter().chain(snos_batches_open).collect();

    assert!(!all_batches.is_empty(), "Expected batches to be created");

    // Assert: Batches can span up to 15 blocks (not constrained by aggregator batches)
    for batch in &all_batches {
        assert_eq!(batch.aggregator_batch_index, None, "L3 batches should have no aggregator link");

        // In L2 mode, if an aggregator batch covered blocks 0-9, a SNOS batch couldn't span 0-14
        // But in L3 mode, it can span any range up to max_blocks_per_snos_batch
        if batch.start_block == 0 {
            // This batch could potentially span more than 10 blocks (which would be an aggregator boundary in L2)
            // The only constraints are max_blocks_per_snos_batch (15) and other limits
            assert!(
                batch.num_blocks <= 15,
                "Batch should respect max_blocks_per_snos_batch limit, got {}",
                batch.num_blocks
            );
        }
    }

    Ok(())
}
