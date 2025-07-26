use std::error::Error;

use crate::core::client::database::MockDatabaseClient;
use crate::core::client::storage::MockStorageClient;
use crate::tests::config::TestConfigBuilder;
use crate::worker::event_handler::triggers::JobTrigger;
use httpmock::MockServer;
use orchestrator_prover_client_interface::MockProverClient;
use rstest::rstest;
use serde_json::json;
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::JsonRpcClient;
use url::Url;

#[rstest]
#[tokio::test]
#[case(false)]
#[case(true)]
async fn test_batching_worker(#[case] has_existing_batch: bool) -> Result<(), Box<dyn Error>> {
    let server = MockServer::start();
    let mut db = MockDatabaseClient::new();
    let mut storage = MockStorageClient::new();
    let start_block;
    let end_block;

    // Mocking database expectations
    if !has_existing_batch {
        // DB does not have an existing batch
        // Returning None to specify no existing batch
        db.expect_get_latest_batch().returning(|| Ok(None));

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
            ..Default::default()
        };
        // Returning Some(existing_batch) to specify an existing batch
        db.expect_get_latest_batch().returning(move || Ok(Some(existing_batch.clone())));

        // Batch containing blocks from 4 to 7
        start_block = 4;
        end_block = 7;
    }

    // Mock block number response
    let rpc_response_block_number = end_block;
    let response = json!({ "id": 1, "jsonrpc": "2.0", "result": rpc_response_block_number });

    // Mock state update responses for each block
    for block_num in start_block..end_block + 1 {
        // Mock storage expectations for each block
        let state_update_path = format!("state_update/batch/{}.json", if has_existing_batch { 2 } else { 1 });
        let state_update_path_clone = state_update_path.clone();
        storage.expect_put_data().withf(move |_, path| path == state_update_path_clone).returning(|_, _| Ok(()));

        // Mock database expectations for each block
        if block_num == start_block {
            db.expect_create_batch()
                .withf(move |batch| {
                    batch.start_block == block_num
                        && batch.end_block == block_num
                        && batch.num_blocks == 1
                        && batch.squashed_state_updates_path == state_update_path
                })
                .returning(Ok);
        } else {
            db.expect_update_or_create_batch()
                .withf(move |batch, updates| {
                    batch.end_block == block_num - 1
                        && updates.end_block == Some(block_num)
                        && updates.is_batch_ready == Some(false)
                })
                .returning(|batch, _| Ok(batch.clone()));
        }
    }

    let provider = JsonRpcClient::new(HttpTransport::new(
        Url::parse(format!("http://localhost:{}", server.port()).as_str()).expect("Failed to parse URL"),
    ));

    let mut prover_client = MockProverClient::new();
    prover_client.expect_submit_task().times(1).returning(|_| Ok("bucket_id".to_string()));

    let services = TestConfigBuilder::new()
        .configure_starknet_client(provider.into())
        .configure_database(db.into())
        .configure_storage_client(storage.into())
        .configure_prover_client(prover_client.into())
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
