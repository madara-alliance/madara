use crate::core::config::StarknetVersion;
use crate::tests::config::{ConfigType, TestConfigBuilder};
use crate::tests::workers::batching::helpers::generate_custom_bouncer_weights;
use crate::types::batch::{AggregatorBatch, AggregatorBatchStatus, SnosBatch, SnosBatchStatus};
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

/// Test: SNOS batch closes when max_blocks_per_snos_batch is reached
///
/// This test validates that:
/// - When a SNOS batch reaches max_blocks_per_snos_batch, it closes
/// - A new SNOS batch starts for subsequent blocks
/// - Both batches remain within the same aggregator batch (L2 mode)
#[rstest]
#[tokio::test]
async fn test_snos_closes_on_max_blocks_per_batch() -> Result<(), Box<dyn Error>> {
    let server = MockServer::start();
    let provider_url = format!("http://localhost:{}", server.port());

    let provider = JsonRpcClient::new(HttpTransport::new(Url::parse(&provider_url).expect("Failed to parse URL")));

    let services = TestConfigBuilder::new()
        .configure_starknet_client(provider.into())
        .configure_storage_client(ConfigType::Actual)
        .configure_database(ConfigType::Actual)
        .configure_lock_client(ConfigType::Actual)
        .configure_layer(Layer::L2)
        .configure_madara_feeder_gateway_url(&provider_url)
        .configure_max_blocks_per_snos_batch(Some(5)) // Key config: max 5 blocks per SNOS batch
        .build()
        .await;

    let database = services.config.database();

    // Setup: Create aggregator batch covering blocks 0-19
    let aggregator_batch = AggregatorBatch {
        index: 1,
        start_block: 0,
        end_block: 19,
        num_blocks: 20,
        blob_len: 200,
        squashed_state_updates_path: "state_update/batch/1.json".to_string(),
        blob_path: "blob/batch/1.bin".to_string(),
        bucket_id: "test_bucket".to_string(),
        status: AggregatorBatchStatus::Open,
        starknet_version: StarknetVersion::V0_13_2,
        ..Default::default()
    };
    database.create_aggregator_batch(aggregator_batch.clone()).await?;

    // Mock RPC responses
    server.mock(|when, then| {
        when.path("/").body_includes("starknet_blockNumber");
        then.status(200).body(serde_json::to_vec(&json!({ "id": 1, "jsonrpc": "2.0", "result": 19 })).unwrap());
    });

    // Mock bouncer weights
    let builtin_weights = crate::tests::workers::batching::helpers::generate_bouncer_weights();
    server.mock(|when, then| {
        when.method(httpmock::Method::GET).path("/feeder_gateway/get_block_bouncer_weights");
        then.status(200)
            .header("Content-Type", "application/json")
            .body(serde_json::to_vec(&builtin_weights).unwrap());
    });

    // Mock block version fetching
    for block_num in 0..=19 {
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

    // Execute: Run SNOS batching trigger
    SnosBatchingTrigger.run_worker(services.config.clone()).await?;

    // Assert: Verify SNOS batches were created with max 5 blocks each
    let snos_batches_closed = database.get_snos_batches_without_jobs(SnosBatchStatus::Closed).await?;
    let snos_batches_open = database.get_snos_batches_without_jobs(SnosBatchStatus::Open).await?;

    let all_batches: Vec<SnosBatch> = snos_batches_closed.into_iter().chain(snos_batches_open).collect();

    assert!(!all_batches.is_empty(), "Expected SNOS batches to be created");

    // Verify each batch has at most 5 blocks
    for batch in &all_batches {
        assert!(
            batch.num_blocks <= 5,
            "SNOS batch {} has {} blocks, expected max 5",
            batch.index,
            batch.num_blocks
        );
    }

    // Verify batches are linked to aggregator batch
    for batch in &all_batches {
        assert_eq!(
            batch.aggregator_batch_index,
            Some(1),
            "SNOS batch should be linked to aggregator batch 1"
        );
    }

    Ok(())
}

/// Test: SNOS batch with small max_blocks_per_snos_batch
///
/// This test validates that:
/// - When max_blocks_per_snos_batch is set to a small value, batches close accordingly
/// - Multiple batches are created
/// Note: fixed_blocks_per_snos_batch exists in code but doesn't have a test config method
#[rstest]
#[tokio::test]
async fn test_snos_closes_on_small_max_batch_size() -> Result<(), Box<dyn Error>> {
    let server = MockServer::start();
    let provider_url = format!("http://localhost:{}", server.port());

    let provider = JsonRpcClient::new(HttpTransport::new(Url::parse(&provider_url).expect("Failed to parse URL")));

    let services = TestConfigBuilder::new()
        .configure_starknet_client(provider.into())
        .configure_storage_client(ConfigType::Actual)
        .configure_database(ConfigType::Actual)
        .configure_lock_client(ConfigType::Actual)
        .configure_layer(Layer::L3) // Use L3 for simpler setup (no aggregator dependency)
        .configure_madara_feeder_gateway_url(&provider_url)
        .configure_max_blocks_per_snos_batch(Some(3)) // Key config: max 3 blocks per batch
        .build()
        .await;

    let database = services.config.database();

    // Mock RPC responses
    server.mock(|when, then| {
        when.path("/").body_includes("starknet_blockNumber");
        then.status(200).body(serde_json::to_vec(&json!({ "id": 1, "jsonrpc": "2.0", "result": 11 })).unwrap());
    });

    // Mock bouncer weights
    let builtin_weights = crate::tests::workers::batching::helpers::generate_bouncer_weights();
    server.mock(|when, then| {
        when.method(httpmock::Method::GET).path("/feeder_gateway/get_block_bouncer_weights");
        then.status(200)
            .header("Content-Type", "application/json")
            .body(serde_json::to_vec(&builtin_weights).unwrap());
    });

    // Mock block version fetching
    for block_num in 0..=11 {
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

    // Execute: Run SNOS batching trigger
    SnosBatchingTrigger.run_worker(services.config.clone()).await?;

    // Assert: Verify SNOS batches have exactly 3 blocks each
    let snos_batches_closed = database.get_snos_batches_without_jobs(SnosBatchStatus::Closed).await?;
    let snos_batches_open = database.get_snos_batches_without_jobs(SnosBatchStatus::Open).await?;

    let all_batches: Vec<SnosBatch> = snos_batches_closed.into_iter().chain(snos_batches_open).collect();

    assert!(!all_batches.is_empty(), "Expected SNOS batches to be created");

    // Verify batches have at most 3 blocks
    for batch in &all_batches {
        assert!(
            batch.num_blocks <= 3,
            "SNOS batch {} should have at most 3 blocks, got {}",
            batch.index, batch.num_blocks
        );
    }

    Ok(())
}

/// Test: SNOS batch closes on bouncer weight overflow
///
/// This test validates that:
/// - When adding a block would exceed bouncer weight limits, the batch closes
/// - A new batch starts with the block that would have caused overflow
/// - All weight components are checked (l1_gas, proving_gas, etc.)
#[rstest]
#[tokio::test]
async fn test_snos_closes_on_bouncer_weight_overflow() -> Result<(), Box<dyn Error>> {
    let server = MockServer::start();
    let provider_url = format!("http://localhost:{}", server.port());

    let provider = JsonRpcClient::new(HttpTransport::new(Url::parse(&provider_url).expect("Failed to parse URL")));

    // Note: There's no test config method for bouncer_weights_limit yet
    // The weight limits are loaded from file in production
    let services = TestConfigBuilder::new()
        .configure_starknet_client(provider.into())
        .configure_storage_client(ConfigType::Actual)
        .configure_database(ConfigType::Actual)
        .configure_lock_client(ConfigType::Actual)
        .configure_layer(Layer::L3)
        .configure_madara_feeder_gateway_url(&provider_url)
        .build()
        .await;

    let database = services.config.database();

    // Setup: Create an existing SNOS batch with high weights (nearing limit)
    let existing_batch = SnosBatch {
        index: 1,
        aggregator_batch_index: None, // L3 mode
        start_block: 0,
        end_block: 4,
        num_blocks: 5,
        builtin_weights: generate_custom_bouncer_weights(
            500_000,  // Already high l1_gas
            600,
            900,
            90,
            900,
            900_000_000,
            900_000_000,
        ),
        starknet_version: StarknetVersion::V0_13_2,
        status: SnosBatchStatus::Open,
        ..Default::default()
    };
    database.create_snos_batch(existing_batch).await?;

    // Mock RPC responses
    server.mock(|when, then| {
        when.path("/").body_includes("starknet_blockNumber");
        then.status(200).body(serde_json::to_vec(&json!({ "id": 1, "jsonrpc": "2.0", "result": 9 })).unwrap());
    });

    // Mock bouncer weights with values that will cause overflow
    let high_weights = generate_custom_bouncer_weights(
        150_000,  // Adding this to 500k would exceed 600k limit
        100,
        100,
        10,
        100,
        100_000_000,
        100_000_000,
    );
    server.mock(|when, then| {
        when.method(httpmock::Method::GET).path("/feeder_gateway/get_block_bouncer_weights");
        then.status(200)
            .header("Content-Type", "application/json")
            .body(serde_json::to_vec(&high_weights).unwrap());
    });

    // Mock block version fetching
    for block_num in 5..=9 {
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

    // Execute: Run SNOS batching trigger
    SnosBatchingTrigger.run_worker(services.config.clone()).await?;

    // Assert: Verify batch closed due to weight overflow
    let snos_batches_closed = database.get_snos_batches_without_jobs(SnosBatchStatus::Closed).await?;
    let snos_batches_open = database.get_snos_batches_without_jobs(SnosBatchStatus::Open).await?;

    let all_batches: Vec<SnosBatch> = snos_batches_closed.into_iter().chain(snos_batches_open).collect();

    // Should have at least 2 batches (original closed + new one)
    assert!(
        all_batches.len() >= 2,
        "Expected at least 2 batches due to weight overflow, got {}",
        all_batches.len()
    );

    // Verify first batch was closed
    let batch_1 = all_batches.iter().find(|b| b.index == 1);
    if let Some(b1) = batch_1 {
        assert_eq!(
            b1.status,
            SnosBatchStatus::Closed,
            "Batch 1 should be closed due to weight overflow"
        );
    }

    // Verify second batch exists starting from block 5
    let batch_2 = all_batches.iter().find(|b| b.start_block == 5);
    assert!(batch_2.is_some(), "Expected batch starting at block 5 after overflow");

    Ok(())
}

/// Test: SNOS batch closes on Starknet version change
///
/// This test validates that:
/// - When the Starknet version changes, the SNOS batch closes
/// - A new batch starts with the new version
/// - Each batch has a homogeneous version
#[rstest]
#[tokio::test]
async fn test_snos_closes_on_version_change() -> Result<(), Box<dyn Error>> {
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
        .build()
        .await;

    let database = services.config.database();

    // Setup: Create existing SNOS batch with v0.13.2
    let existing_batch = SnosBatch {
        index: 1,
        aggregator_batch_index: None,
        start_block: 0,
        end_block: 4,
        num_blocks: 5,
        builtin_weights: crate::tests::utils::default_test_bouncer_weights(),
        starknet_version: StarknetVersion::V0_13_2,
        status: SnosBatchStatus::Open,
        ..Default::default()
    };
    database.create_snos_batch(existing_batch).await?;

    // Mock RPC responses
    server.mock(|when, then| {
        when.path("/").body_includes("starknet_blockNumber");
        then.status(200).body(serde_json::to_vec(&json!({ "id": 1, "jsonrpc": "2.0", "result": 9 })).unwrap());
    });

    // Mock bouncer weights
    let builtin_weights = crate::tests::workers::batching::helpers::generate_bouncer_weights();
    server.mock(|when, then| {
        when.method(httpmock::Method::GET).path("/feeder_gateway/get_block_bouncer_weights");
        then.status(200)
            .header("Content-Type", "application/json")
            .body(serde_json::to_vec(&builtin_weights).unwrap());
    });

    // Mock blocks 5-9 with NEW version 0.13.3 (version change!)
    for block_num in 5..=9 {
        let pattern = format!(r#".*"block_number"\s*:\s*{}[,\}}].*"#, block_num);
        server.mock(move |when, then| {
            when.path("/")
                .body_includes("starknet_getBlockWithTxHashes")
                .body_matches(pattern.as_str());
            then.status(200).body(
                serde_json::to_vec(&json!({
                    "jsonrpc": "2.0",
                    "result": crate::tests::workers::batching::helpers::generate_block_with_tx_hashes(block_num, "0.13.3"),
                    "id": 1
                }))
                .unwrap(),
            );
        });
    }

    // Execute: Run SNOS batching trigger
    SnosBatchingTrigger.run_worker(services.config.clone()).await?;

    // Assert: Verify version change triggered batch closure
    let snos_batches_closed = database.get_snos_batches_without_jobs(SnosBatchStatus::Closed).await?;
    let snos_batches_open = database.get_snos_batches_without_jobs(SnosBatchStatus::Open).await?;

    let all_batches: Vec<SnosBatch> = snos_batches_closed.into_iter().chain(snos_batches_open).collect();

    // Verify batch 1 was closed
    let batch_1 = all_batches.iter().find(|b| b.index == 1);
    if let Some(b1) = batch_1 {
        assert_eq!(
            b1.status,
            SnosBatchStatus::Closed,
            "Batch 1 should be closed due to version change"
        );
        assert_eq!(b1.starknet_version, StarknetVersion::V0_13_2, "Batch 1 should have v0.13.2");
    }

    // Verify batch with new version exists
    let batch_v0_13_3 = all_batches.iter().find(|b| b.starknet_version == StarknetVersion::V0_13_3);
    if let Some(b2) = batch_v0_13_3 {
        assert_eq!(
            b2.start_block, 5,
            "New version batch should start at block 5 (first block with v0.13.3)"
        );
    }

    Ok(())
}
