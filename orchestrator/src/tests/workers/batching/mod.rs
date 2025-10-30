use crate::core::client::database::MockDatabaseClient;
use crate::core::client::lock::{LockResult, LockValue, MockLockClient};
use crate::core::client::storage::MockStorageClient;
use crate::tests::config::{ConfigType, TestConfigBuilder};
use crate::types::batch::SnosBatchStatus;
use crate::worker::event_handler::triggers::JobTrigger;
use blockifier::bouncer::BouncerWeights;
use bytes::Bytes;
use httpmock::MockServer;
use num_traits::FromPrimitive;
use orchestrator_prover_client_interface::MockProverClient;
use orchestrator_utils::layer::Layer;
use rstest::rstest;
use serde_json::json;
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::JsonRpcClient;
use starknet_api::execution_resources::GasAmount;
use starknet_core::types::{Felt, MaybePreConfirmedStateUpdate, StateDiff, StateUpdate};
use std::error::Error;
use std::sync::{Arc, Mutex};
use url::Url;

#[rstest]
#[case(false)]
#[case(true)]
#[tokio::test]
async fn test_batching_worker(#[case] has_existing_batch: bool) -> Result<(), Box<dyn Error>> {
    let server = MockServer::start();
    let mut database = MockDatabaseClient::new();
    let mut storage = MockStorageClient::new();
    let mut lock = MockLockClient::new();
    let start_block;
    let end_block;

    // Mocking database expectations
    if !has_existing_batch {
        // DB does not have existing batches
        // Returning None for both aggregator and SNOS batches
        database.expect_get_latest_aggregator_batch().returning(|| Ok(None));
        database.expect_get_latest_snos_batch().returning(|| Ok(None));

        // Batch containing blocks from 0 to 5
        start_block = 0;
        end_block = 5;
    } else {
        // DB does have existing batches
        let existing_aggregator_batch = crate::types::batch::AggregatorBatch {
            index: 1,
            start_block: 0,
            end_block: 3,
            num_blocks: 4,
            squashed_state_updates_path: "state_update/batch/1.json".to_string(),
            is_batch_ready: false,
            created_at: chrono::Utc::now(),
            starknet_version: "0.13.2".to_string(),
            ..Default::default()
        };

        let existing_snos_batch = crate::types::batch::SnosBatch {
            snos_batch_id: 1,
            aggregator_batch_index: Some(1),
            start_block: 0,
            end_block: 3,
            num_blocks: 4,
            status: crate::types::batch::SnosBatchStatus::Open,
            created_at: chrono::Utc::now(),
            ..Default::default()
        };

        // Returning existing batches
        database.expect_get_latest_aggregator_batch().returning(move || Ok(Some(existing_aggregator_batch.clone())));
        database.expect_get_latest_snos_batch().returning(move || Ok(Some(existing_snos_batch.clone())));

        // Batch containing blocks from 4 to 7
        start_block = 4;
        end_block = 7;
    }

    // Mock storage expectation for storing data
    storage.expect_put_data().returning(|_, _| Ok(()));

    // Mock database expectations for batching
    database.expect_create_aggregator_batch().returning(Ok);
    database.expect_update_or_create_aggregator_batch().returning(|batch, _| Ok(batch.clone()));

    // Mock SNOS batch operations
    database.expect_create_snos_batch().returning(Ok);
    database.expect_update_or_create_snos_batch().returning(|batch, _| Ok(batch.clone()));
    database.expect_get_next_snos_batch_id().returning(|| Ok(1));
    database.expect_get_open_snos_batches_by_aggregator_index().returning(|_| Ok(vec![]));
    database.expect_close_all_snos_batches_for_aggregator().returning(|_| Ok(vec![]));

    if has_existing_batch {
        storage.expect_get_data().returning(|_| Ok(Bytes::from(get_dummy_state_update(1).to_string())));
    }

    // Mock lock client response
    lock.expect_acquire_lock()
        .withf(move |key, value, expiry_seconds, owner| {
            key == "BatchingWorker" && *value == LockValue::Boolean(false) && *expiry_seconds == 3600 && owner.is_none()
        })
        .returning(|_, _, _, _| Ok(LockResult::Acquired));

    lock.expect_release_lock()
        .withf(move |key, owner| key == "BatchingWorker" && owner.is_none())
        .returning(|_, _| Ok(LockResult::Released));

    let rpc_block_call_mock = server.mock(|when, then| {
        when.path("/").body_includes("starknet_blockNumber");
        then.status(200).body(serde_json::to_vec(&json!({ "id": 1, "jsonrpc": "2.0", "result": end_block })).unwrap());
    });

    // Mock starknet_getBlockWithTxHashes for version fetching
    for block_num in start_block..=end_block {
        let pattern = format!(r#".*"block_number"\s*:\s*{}[,\}}].*"#, block_num);
        server.mock(move |when, then| {
            when.path("/").body_includes("starknet_getBlockWithTxHashes").body_matches(pattern.as_str());
            then.status(200).body(
                serde_json::to_vec(&json!({
                    "jsonrpc":"2.0",
                    "result": get_dummy_block_with_tx_hashes(block_num, "0.13.2"),
                    "id":1
                }))
                .unwrap(),
            );
        });
    }

    // Mock state update calls for each block
    for block_num in start_block..=end_block {
        let state_update = get_dummy_state_update(block_num);
        server.mock(|when, then| {
            when.path("/").body_includes("starknet_getStateUpdate");
            then.status(200)
                .body(serde_json::to_vec(&json!({ "id": 1,"jsonrpc":"2.0","result": state_update })).unwrap());
        });
    }

    // NOW create the provider and config after all mocks are set up
    let provider = JsonRpcClient::new(HttpTransport::new(
        Url::parse(format!("http://localhost:{}", server.port()).as_str()).expect("Failed to parse URL"),
    ));

    let mut prover_client = MockProverClient::new();
    if !has_existing_batch {
        prover_client.expect_submit_task().times(1).returning(|_| Ok("bucket_id".to_string()));
    }

    let services = TestConfigBuilder::new()
        .configure_starknet_client(provider.into())
        .configure_database(database.into())
        .configure_storage_client(storage.into())
        .configure_prover_client(prover_client.into())
        .configure_lock_client(lock.into())
        .build()
        .await;

    crate::worker::event_handler::triggers::batching::BatchingTrigger.run_worker(services.config).await?;

    rpc_block_call_mock.assert();

    Ok(())
}

/// Tests that the batching worker correctly creates separate batches when Starknet version changes.
#[rstest]
#[tokio::test]
async fn test_batching_worker_with_multiple_blocks() -> Result<(), Box<dyn Error>> {
    let server = MockServer::start();
    let mut database = MockDatabaseClient::new();
    let mut storage = MockStorageClient::new();
    let mut lock = MockLockClient::new();

    let existing_aggregator_batch = crate::types::batch::AggregatorBatch {
        index: 1,
        start_block: 0,
        end_block: 3,
        num_blocks: 4,
        squashed_state_updates_path: "state_update/batch/1.json".to_string(),
        is_batch_ready: true,
        created_at: chrono::Utc::now(),
        starknet_version: "0.13.2".to_string(),
        ..Default::default()
    };

    let existing_snos_batch = crate::types::batch::SnosBatch {
        snos_batch_id: 1,
        aggregator_batch_index: Some(1),
        start_block: 0,
        end_block: 3,
        num_blocks: 4,
        status: crate::types::batch::SnosBatchStatus::Closed,
        created_at: chrono::Utc::now(),
        ..Default::default()
    };

    database.expect_get_latest_aggregator_batch().returning(move || Ok(Some(existing_aggregator_batch.clone())));
    database.expect_get_latest_snos_batch().returning(move || Ok(Some(existing_snos_batch.clone())));

    storage.expect_get_data().returning(|_| Ok(Bytes::from(get_dummy_state_update(1).to_string())));

    storage.expect_put_data().returning(|_, _| Ok(()));

    let batches_updated = Arc::new(Mutex::new(Vec::new()));
    let batches_updated_clone = batches_updated.clone();
    database.expect_update_or_create_aggregator_batch().returning(move |batch, _| {
        batches_updated_clone.lock().unwrap().push(batch.clone());
        Ok(batch.clone())
    });

    database.expect_create_aggregator_batch().returning(Ok);

    // Mock SNOS batch operations
    database.expect_create_snos_batch().returning(Ok);
    database.expect_update_or_create_snos_batch().returning(|batch, _| Ok(batch.clone()));
    database.expect_get_next_snos_batch_id().returning(|| Ok(2));
    database.expect_get_open_snos_batches_by_aggregator_index().returning(|_| Ok(vec![]));
    database.expect_close_all_snos_batches_for_aggregator().returning(|_| Ok(vec![]));

    lock.expect_acquire_lock()
        .withf(move |key, value, expiry_seconds, owner| {
            key == "BatchingWorker" && *value == LockValue::Boolean(false) && *expiry_seconds == 3600 && owner.is_none()
        })
        .returning(|_, _, _, _| Ok(LockResult::Acquired));

    lock.expect_release_lock()
        .withf(move |key, owner| key == "BatchingWorker" && owner.is_none())
        .returning(|_, _| Ok(LockResult::Released));

    // Mock starknet_blockNumber - single mock that's reusable
    server.mock(|when, then| {
        when.path("/").body_includes("starknet_blockNumber");
        then.status(200).body(serde_json::to_vec(&json!({ "id": 1, "jsonrpc": "2.0", "result": 7 })).unwrap());
    });

    // Mock starknet_getBlockWithTxHashes - use body_matches with regex for precise matching
    server.mock(|when, then| {
        when.path("/")
            .body_includes("starknet_getBlockWithTxHashes")
            .body_matches(r#".*"block_number"\s*:\s*4[,\}].*"#);
        then.status(200).body(
            serde_json::to_vec(&json!({
                "jsonrpc":"2.0",
                "result": get_dummy_block_with_tx_hashes(4, "0.13.2"),
                "id":1
            }))
            .unwrap(),
        );
    });

    server.mock(|when, then| {
        when.path("/")
            .body_includes("starknet_getBlockWithTxHashes")
            .body_matches(r#".*"block_number"\s*:\s*5[,\}].*"#);
        then.status(200).body(
            serde_json::to_vec(&json!({
                "jsonrpc":"2.0",
                "result": get_dummy_block_with_tx_hashes(5, "0.13.2"),
                "id":1
            }))
            .unwrap(),
        );
    });

    server.mock(|when, then| {
        when.path("/")
            .body_includes("starknet_getBlockWithTxHashes")
            .body_matches(r#".*"block_number"\s*:\s*6[,\}].*"#);
        then.status(200).body(
            serde_json::to_vec(&json!({
                "jsonrpc":"2.0",
                "result": get_dummy_block_with_tx_hashes(6, "0.13.3"),
                "id":1
            }))
            .unwrap(),
        );
    });

    server.mock(|when, then| {
        when.path("/")
            .body_includes("starknet_getBlockWithTxHashes")
            .body_matches(r#".*"block_number"\s*:\s*7[,\}].*"#);
        then.status(200).body(
            serde_json::to_vec(&json!({
                "jsonrpc":"2.0",
                "result": get_dummy_block_with_tx_hashes(7, "0.13.3"),
                "id":1
            }))
            .unwrap(),
        );
    });

    // Mock starknet_getStateUpdate - separate mock for each block
    for block_num in 4..=7 {
        let state_update = get_dummy_state_update(block_num);
        server.mock(|when, then| {
            when.path("/").body_includes("starknet_getStateUpdate");
            then.status(200).body(serde_json::to_vec(&json!({"jsonrpc":"2.0","result":state_update,"id":1})).unwrap());
        });
    }

    let provider = JsonRpcClient::new(HttpTransport::new(
        Url::parse(format!("http://localhost:{}", server.port()).as_str()).expect("Failed to parse URL"),
    ));

    let mut prover_client = MockProverClient::new();
    prover_client.expect_submit_task().times(2).returning(|_| Ok("new_bucket_id".to_string()));

    let services = TestConfigBuilder::new()
        .configure_starknet_client(provider.into())
        .configure_database(database.into())
        .configure_storage_client(storage.into())
        .configure_prover_client(prover_client.into())
        .configure_lock_client(lock.into())
        .configure_min_block_to_process(0)
        .configure_max_block_to_process(Some(10))
        .build()
        .await;

    crate::worker::event_handler::triggers::batching::BatchingTrigger.run_worker(services.config).await?;

    let updated_batches = batches_updated.lock().unwrap();

    assert!(!updated_batches.is_empty(), "Expected at least one batch to be updated when processing blocks");

    assert_eq!(
        updated_batches.len(),
        2,
        "Expected exactly 2 batches to be created (one for each version), but found {}",
        updated_batches.len()
    );

    let batch_2 = updated_batches.iter().find(|b| b.index == 2).expect("Batch 2 should exist");
    assert_eq!(batch_2.start_block, 4, "Batch 2 should start at block 4");
    assert_eq!(batch_2.end_block, 5, "Batch 2 should end at block 5");
    assert_eq!(batch_2.starknet_version, "0.13.2", "Batch 2 should have version 0.13.2");

    let batch_3 = updated_batches.iter().find(|b| b.index == 3).expect("Batch 3 should exist");
    assert_eq!(batch_3.start_block, 6, "Batch 3 should start at block 6");
    assert_eq!(batch_3.end_block, 7, "Batch 3 should end at block 7");
    assert_eq!(batch_3.starknet_version, "0.13.3", "Batch 3 should have version 0.13.3");

    Ok(())
}

/// Test the batching worker for L3s.
/// Doesn't mock Database or Storage.
/// Mock `madara_V0_1_0_getBlockBuiltinWeights` response.
/// NOTE: This method is present only in Madara as of now.
#[rstest]
#[case(false)]
#[case(true)]
#[tokio::test]
async fn test_batching_worker_l3(#[case] has_existing_batch: bool) -> Result<(), Box<dyn Error>> {
    let server = MockServer::start();

    let end_block;
    let provider_url = format!("http://localhost:{}", server.port());

    let provider = JsonRpcClient::new(HttpTransport::new(Url::parse(&provider_url).expect("Failed to parse URL")));

    let services = TestConfigBuilder::new()
        .configure_starknet_client(provider.into())
        .configure_storage_client(ConfigType::Actual)
        .configure_database(ConfigType::Actual)
        .configure_lock_client(ConfigType::Actual)
        .configure_layer(Layer::L3)
        .configure_madara_admin_rpc_url(&provider_url)
        .configure_max_blocks_per_snos_batch(None)
        .build()
        .await;

    let database = services.config.database();

    if !has_existing_batch {
        end_block = 11;
    } else {
        let existing_snos_batch = crate::types::batch::SnosBatch {
            snos_batch_id: 1,
            aggregator_batch_index: None,
            start_block: 0,
            end_block: 3,
            num_blocks: 4,
            status: SnosBatchStatus::Open,
            created_at: chrono::Utc::now(),
            ..Default::default()
        };
        database.create_snos_batch(existing_snos_batch).await?;

        end_block = 14;
    }

    // Mock block number call
    let rpc_block_call_mock = server.mock(|when, then| {
        when.path("/").body_includes("starknet_blockNumber");
        then.status(200).body(serde_json::to_vec(&json!({ "id": 1, "jsonrpc": "2.0", "result": end_block })).unwrap());
    });

    // Mock builtin weights calls for each block
    let builtin_weights = get_dummy_builtin_weights();
    server.mock(|when, then| {
        when.path("/").body_includes("madara_V0_1_0_getBlockBuiltinWeights");
        then.status(200)
            .body(serde_json::to_vec(&json!({ "id": 1,"jsonrpc":"2.0","result": builtin_weights })).unwrap());
    });

    crate::worker::event_handler::triggers::batching::BatchingTrigger.run_worker(services.config.clone()).await?;

    let snos_batches_closed = database.get_snos_batches_without_jobs(SnosBatchStatus::Closed).await?;
    let snos_batches_open = database.get_snos_batches_without_jobs(SnosBatchStatus::Open).await?;

    assert_eq!(snos_batches_closed.len(), if has_existing_batch { 3 } else { 2 });
    assert_eq!(snos_batches_open.len(), 1);

    rpc_block_call_mock.assert();

    Ok(())
}

fn get_dummy_block_with_tx_hashes(block_num: u64, starknet_version: &str) -> serde_json::Value {
    json!({
        "status": "ACCEPTED_ON_L1",
        "block_hash": format!("0x{:x}", block_num),
        "parent_hash": format!("0x{:x}", block_num.saturating_sub(1)),
        "block_number": block_num,
        "new_root": format!("0x{:x}", block_num + 1),
        "timestamp": 1234567890 + block_num,
        "sequencer_address": "0x0",
        "l1_gas_price": {
            "price_in_fri": "0x1",
            "price_in_wei": "0x1"
        },
        "l2_gas_price": {
            "price_in_fri": "0x1",
            "price_in_wei": "0x1"
        },
        "l1_data_gas_price": {
            "price_in_fri": "0x1",
            "price_in_wei": "0x1"
        },
        "l1_da_mode": "CALLDATA",
        "starknet_version": starknet_version,
        "transactions": []
    })
}

fn get_dummy_state_update(block_num: u64) -> serde_json::Value {
    let state_update = MaybePreConfirmedStateUpdate::Update(StateUpdate {
        block_hash: Felt::from_u64(block_num).unwrap(),
        new_root: Felt::from_u64(block_num + 1).unwrap(),
        old_root: Felt::from_u64(block_num).unwrap(),
        state_diff: StateDiff {
            storage_diffs: vec![],
            deprecated_declared_classes: vec![],
            declared_classes: vec![],
            deployed_contracts: vec![],
            replaced_classes: vec![],
            nonces: vec![],
        },
    });

    serde_json::to_value(&state_update).unwrap()
}

fn get_dummy_builtin_weights() -> serde_json::Value {
    let response = BouncerWeights {
        l1_gas: 500_000,
        message_segment_length: 700,
        n_events: 1000,
        n_txs: 100,
        state_diff_size: 1000,
        sierra_gas: GasAmount(1_000_000_000),
        proving_gas: GasAmount(1_100_000_000),
    };

    serde_json::to_value(response).unwrap()
}
