use crate::MadaraCmdBuilder;
use rstest::rstest;
use serde_json::json;
use starknet_providers::Provider;

#[rstest]
#[tokio::test]
async fn test_storage_proof() {
    let _ = env_logger::builder().is_test(true).try_init();

    let cmd_builder = MadaraCmdBuilder::new().args([
        "--full",
        "-n",
        "sepolia",
        "--no-sync-polling",
        "--n-blocks-to-sync",
        "20",
        "--no-l1-sync",
        // trie log config
        "--db-max-saved-trie-logs",
        "20",
        "--db-max-saved-snapshots",
        "5",
        "--db-snapshot-interval",
        "4",
    ]);

    let mut node = cmd_builder.run();
    node.wait_for_ready().await;
    node.wait_for_sync_to(19).await;

    let client = reqwest::Client::new();
    let ret: serde_json::Value = client.post(node.rpc_url.clone()).json(&json!({
        "jsonrpc": "2.0",
        "method": "starknet_getStorageProof",
        "params": {
            "block_id": { "block_number": 19 },
            "contract_addresses": ["0x9459c8cb7424a2946e6bcf7bc204e349a2865f84f2ae75586ada2897d74c4e"],
            "contracts_storage_keys": [ {
                "contract_address": "0x49d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7",
                "storage_keys": ["0x5d2e9527cbeb1a51aa084b0de7501f343b7b1bf24a0c427d6204a7b7988970", "0x1390569bb0a3a722eb4228e8700301347da081211d5c2ded2db22ef389551ab"]
            } ]
        },
        "id": 1
    })).send().await.unwrap().json().await.unwrap();

    eprintln!("ajshdjkashd {:?}", ret);
}
