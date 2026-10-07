use crate::{MadaraCmd, MadaraCmdBuilder};
use rstest::rstest;
use serde_json::{json, Value};
use starknet_types_core::{
    felt::Felt,
    hash::{Poseidon, StarkHash},
};

const STARKNET_STATE_PREFIX: Felt = Felt::from_hex_unchecked("0x535441524b4e45545f53544154455f5630");

#[rstest]
#[tokio::test]
async fn test_storage_proof_snapshots() {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();

    let cmd_builder = MadaraCmdBuilder::new().args([
        "--full",
        "-n",
        "sepolia",
        "--sync-stop-at",
        "19",
        "--no-l1-sync",
        "--db-max-saved-trie-logs",
        "20",
        "--db-max-kept-snapshots",
        "10000",
        "--db-snapshot-interval",
        "1",
        "--rpc-storage-proof-max-distance",
        "20",
    ]);

    let mut node = cmd_builder.run();
    node.wait_for_ready().await;
    node.wait_for_sync_to(19).await;

    test_storage_proof_inner(node).await;
}

#[rstest]
#[tokio::test]
async fn test_storage_proof_trie_log() {
    let cmd_builder = MadaraCmdBuilder::new().args([
        "--full",
        "-n",
        "sepolia",
        "--sync-stop-at",
        "19",
        "--no-l1-sync",
        "--db-max-saved-trie-logs",
        "20",
        "--db-max-kept-snapshots",
        "5",
        "--db-snapshot-interval",
        "4",
        "--rpc-storage-proof-max-distance",
        "20",
    ]);

    let mut node = cmd_builder.run();
    node.wait_for_ready().await;
    node.wait_for_sync_to(19).await;

    test_storage_proof_inner(node).await;
}

async fn get_storage_proof(node: &MadaraCmd, params: Value) -> Value {
    reqwest::Client::new()
        .post(node.rpc_url.clone().unwrap())
        .json(&json!({
            "jsonrpc": "2.0",
            "method": "starknet_getStorageProof",
            "params": params,
            "id": 1,
        }))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap()["result"]
        .clone()
}

fn assert_canonical_roots(
    proof: &Value,
    expected_block_hash: &str,
    expected_contract_root: &str,
    expected_class_root: &str,
    expected_state_root: &str,
) {
    let roots = &proof["global_roots"];
    assert_eq!(roots["block_hash"], expected_block_hash);
    assert_eq!(roots["contracts_tree_root"], expected_contract_root);
    assert_eq!(roots["classes_tree_root"], expected_class_root);

    let contract_root = Felt::from_hex(expected_contract_root).unwrap();
    let class_root = Felt::from_hex(expected_class_root).unwrap();
    let state_root = if class_root == Felt::ZERO {
        contract_root
    } else {
        Poseidon::hash_array(&[STARKNET_STATE_PREFIX, contract_root, class_root])
    };
    assert_eq!(state_root, Felt::from_hex(expected_state_root).unwrap());

    assert!(proof["contracts_proof"]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|node| node["node_hash"] == expected_contract_root));
}

async fn test_storage_proof_inner(node: MadaraCmd) {
    let block_18 = get_storage_proof(
        &node,
        json!({
            "block_id": { "block_number": 18 },
            "contract_addresses": ["0x9459c8cb7424a2946e6bcf7bc204e349a2865f84f2ae75586ada2897d74c4e"],
            "contracts_storage_keys": [{
                "contract_address": "0x49d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7",
                "storage_keys": [
                    "0x5d2e9527cbeb1a51aa084b0de7501f343b7b1bf24a0c427d6204a7b7988970",
                    "0x1390569bb0a3a722eb4228e8700301347da081211d5c2ded2db22ef389551ab"
                ]
            }]
        }),
    )
    .await;

    assert_canonical_roots(
        &block_18,
        "0x5beb56c7d9a9fc066e695c3fc467f45532cace83d9979db4ccfd6b77ca476af",
        "0x4196effc79506ae6622b1e38d10dc9d42debf149dd63879b1052db1fd191664",
        "0x6eeb97e12755a43e64901f54239ebcb0a0f9b2b87fca9b91dd7059689df7a9",
        "0x4c7f8d53ff361055ca6b224bff96bae036e1694c7d0efd0bef5fc633eef720d",
    );
    assert_eq!(
        block_18["contracts_proof"]["contract_leaves_data"][0],
        json!({
            "class_hash": "0x348f560344334951bcbccd27aff05a9f6bedeaefc36315fe20e842163adae3d",
            "nonce": "0x0",
            "storage_root": "0x0"
        })
    );
    assert_eq!(block_18["contracts_storage_proofs"].as_array().unwrap().len(), 1);
    assert!(!block_18["contracts_storage_proofs"][0].as_array().unwrap().is_empty());

    let block_5 = get_storage_proof(
        &node,
        json!({
            "block_id": { "block_number": 5 },
            "contract_addresses": ["0x9459c8cb7424a2946e6bcf7bc204e349a2865f000000000000000897d74c4e"]
        }),
    )
    .await;

    assert_canonical_roots(
        &block_5,
        "0x13b390a0b2c48f907cda28c73a12aa31b96d51bc1be004ba5f71174d8d70e4f",
        "0x79c232ec00890309f011037762ed19439ce50c2245ed8934c48e737bcb4f7f0",
        "0x0",
        "0x79c232ec00890309f011037762ed19439ce50c2245ed8934c48e737bcb4f7f0",
    );
    assert!(block_5["classes_proof"].as_array().unwrap().is_empty());
    assert!(block_5["contracts_storage_proofs"].as_array().unwrap().is_empty());
    assert_eq!(
        block_5["contracts_proof"]["contract_leaves_data"][0],
        json!({ "class_hash": "0x0", "nonce": "0x0", "storage_root": "0x0" })
    );
}
