use crate::core::client::database::MockDatabaseClient;
use crate::core::client::lock::{LockResult, LockValue, MockLockClient};
use crate::core::client::storage::MockStorageClient;
use crate::tests::config::TestConfigBuilder;
use crate::worker::event_handler::triggers::JobTrigger;
use bytes::Bytes;
use httpmock::MockServer;
use num_traits::FromPrimitive;
use orchestrator_prover_client_interface::MockProverClient;
use rstest::rstest;
use serde_json::json;
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::JsonRpcClient;
use starknet_core::types::{Felt, MaybePendingStateUpdate, StateDiff, StateUpdate};
use std::error::Error;
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
        // DB does not have an existing batch
        // Returning None to specify no existing batch
        database.expect_get_latest_batch().returning(|| Ok(None));

        // Batch containing blocks from 0 to 5
        start_block = 0;
        end_block = 5;
    } else {
        // DB does have an existing batch
        let existing_batch = crate::types::batch::Batch {
            index: 1,
            start_block: 0,
            end_block: 3,
            num_blocks: 4,
            squashed_state_updates_path: "state_update/batch/1.json".to_string(),
            is_batch_ready: false,
            created_at: chrono::Utc::now(),
            ..Default::default()
        };
        // Returning Some(existing_batch) to specify an existing batch
        database.expect_get_latest_batch().returning(move || Ok(Some(existing_batch.clone())));

        // Batch containing blocks from 4 to 7
        start_block = 4;
        end_block = 7;
    }

    // Mock storage expectation for storing data
    storage.expect_put_data().returning(|_, _| Ok(()));

    // Mock database expectations for batching
    database.expect_create_batch().returning(Ok);
    database.expect_update_or_create_batch().returning(|batch, _| Ok(batch.clone()));

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

    // Mock block number call
    let rpc_block_call_mock = server.mock(|when, then| {
        when.path("/").body_includes("starknet_blockNumber");
        then.status(200).body(serde_json::to_vec(&json!({ "id": 1, "jsonrpc": "2.0", "result": end_block })).unwrap());
    });

    // Mock state update calls for each block
    for block_num in start_block..end_block + 1 {
        let state_update = get_dummy_state_update(block_num);
        server.mock(|when, then| {
            when.path("/").body_includes("starknet_getStateUpdate");
            then.status(200)
                .body(serde_json::to_vec(&json!({ "id": 1,"jsonrpc":"2.0","result": state_update })).unwrap());
        });
    }

    crate::worker::event_handler::triggers::batching::BatchingTrigger.run_worker(services.config).await?;

    rpc_block_call_mock.assert();

    Ok(())
}

fn get_dummy_state_update(block_num: u64) -> serde_json::Value {
    let state_update = MaybePendingStateUpdate::Update(StateUpdate {
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
