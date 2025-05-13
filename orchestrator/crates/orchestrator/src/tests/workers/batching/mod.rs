use std::error::Error;
use std::sync::Arc;

use crate::core::client::database::MockDatabaseClient;
use crate::core::client::storage::MockStorageClient;
use crate::tests::config::TestConfigBuilder;
use crate::worker::event_handler::triggers::JobTrigger;
use httpmock::MockServer;
use mockall::predicate::eq;
use rstest::rstest;
use serde_json::json;
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::JsonRpcClient;
use url::Url;

#[rstest]
#[case(false)]
#[case(true)]
#[tokio::test]
async fn test_batching_worker(#[case] has_existing_batch: bool) -> Result<(), Box<dyn Error>> {
    let server = MockServer::start();
    let mut db = MockDatabaseClient::new();
    let mut storage = MockStorageClient::new();
    let start_block;
    let end_block;

    // Mocking database expectations
    if !has_existing_batch {
        db.expect_get_latest_batch().returning(|| Ok(None));
        start_block = 0;
        end_block = 5;
    } else {
        // Mock existing batch
        let existing_batch = crate::worker::event_handler::jobs::models::Batch {
            index: 1,
            start_block: 0,
            end_block: 3,
            size: 4,
            squashed_state_updates_path: "state_update/batch/1.json".to_string(),
            is_batch_ready: false,
            ..Default::default()
        };
        db.expect_get_latest_batch().returning(move || Ok(Some(existing_batch.clone())));
        start_block = 4;
        end_block = 7;
    }

    // Mock block number response
    let rpc_response_block_number = end_block;
    let response = json!({ "id": 1, "jsonrpc": "2.0", "result": rpc_response_block_number });

    // Mock state update responses for each block
    for block_num in start_block..end_block + 1 {
        let state_update = json!({
            "block_hash": format!("0x{:064x}", block_num),
            "new_root": format!("0x{:064x}", block_num + 1),
            "old_root": format!("0x{:064x}", block_num),
            "state_diff": {
                "storage_diffs": [],
                "deployed_contracts": [],
                "declared_classes": [],
                "deprecated_declared_classes": [],
                "nonces": [],
                "replaced_classes": []
            }
        });

        // Mock storage expectations for each block
        let state_update_path = format!("state_update/batch/{}.json", if has_existing_batch { 2 } else { 1 });
        let state_update_path_clone = state_update_path.clone();
        storage.expect_put_data().withf(move |data, path| path == &state_update_path_clone).returning(|_, _| Ok(()));

        // Mock database expectations for each block
        if block_num == start_block {
            db.expect_create_batch()
                .withf(move |batch| {
                    batch.start_block == block_num
                        && batch.end_block == block_num
                        && batch.size == 1
                        && batch.squashed_state_updates_path == state_update_path
                })
                .returning(|batch| Ok(batch));
        } else {
            db.expect_update_batch()
                .withf(move |batch, updates| {
                    batch.end_block == block_num - 1
                        && updates.end_block == block_num
                        && updates.is_batch_ready == false
                })
                .returning(|batch, update| Ok(batch.clone()));
        }
    }

    let provider = JsonRpcClient::new(HttpTransport::new(
        Url::parse(format!("http://localhost:{}", server.port()).as_str()).expect("Failed to parse URL"),
    ));

    let services = TestConfigBuilder::new()
        .configure_starknet_client(provider.into())
        .configure_database(db.into())
        .configure_storage_client(storage.into())
        .build()
        .await;

    // Mock block number call
    let rpc_block_call_mock = server.mock(|when, then| {
        when.path("/").body_includes("starknet_blockNumber");
        then.status(200).body(serde_json::to_vec(&response).unwrap());
    });

    // Mock state update calls for each block
    for block_num in start_block..end_block + 1 {
        let state_update = json!({
            "block_hash": format!("0x{:064x}", block_num),
            "new_root": format!("0x{:064x}", block_num + 1),
            "old_root": format!("0x{:064x}", block_num),
            "state_diff": {
                "storage_diffs": [],
                "deployed_contracts": [],
                "declared_classes": [],
                "deprecated_declared_classes": [],
                "nonces": [],
                "replaced_classes": []
            }
        });

        server.mock(|when, then| {
            when.path("/").body_includes(format!("starknet_getStateUpdate?blockId={}", block_num));
            then.status(200).body(serde_json::to_vec(&state_update).unwrap());
        });
    }

    crate::worker::event_handler::triggers::batching::BatchingTrigger.run_worker(services.config).await?;

    rpc_block_call_mock.assert();

    Ok(())
}
