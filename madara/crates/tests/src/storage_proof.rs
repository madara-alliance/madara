use crate::{MadaraCmd, MadaraCmdBuilder};
use rstest::rstest;

fn normalize(json: &mut serde_json::Value) {
    match json {
        serde_json::Value::Array(arr) => {
            arr.sort_by_key(|k| k.to_string());
            arr.iter_mut().for_each(normalize)
        }
        serde_json::Value::Object(obj) => obj.values_mut().for_each(normalize),
        _ => {}
    }
}

#[rstest]
#[tokio::test]
async fn test_storage_proof_snapshots() {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();

    // only use snapshots
    let cmd_builder = MadaraCmdBuilder::new().args([
        "--full",
        "-n",
        "sepolia",
        "--sync-stop-at",
        "19",
        "--no-l1-sync",
        // trie log config
        "--db-max-saved-trie-logs",
        "20",
        "--db-max-kept-snapshots",
        "10000",
        "--db-snapshot-interval",
        "1",
        // rpc storage proof config
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
    // use trie log
    let cmd_builder = MadaraCmdBuilder::new().args([
        "--full",
        "-n",
        "sepolia",
        "--sync-stop-at",
        "19",
        "--no-l1-sync",
        // trie log config
        "--db-max-saved-trie-logs",
        "20",
        "--db-max-kept-snapshots",
        "5",
        "--db-snapshot-interval",
        "4",
        // rpc storage proof config
        "--rpc-storage-proof-max-distance",
        "20",
    ]);

    let mut node = cmd_builder.run();
    node.wait_for_ready().await;
    node.wait_for_sync_to(19).await;

    test_storage_proof_inner(node).await;
}

async fn test_storage_proof_inner(node: MadaraCmd) {
    let client = reqwest::Client::new();
    let mut ret: serde_json::Value = client.post(node.rpc_url.clone().unwrap()).json(&serde_json::json!({
        "jsonrpc": "2.0",
        "method": "starknet_getStorageProof",
        "params": {
            "block_id": { "block_number": 18 },
            "contract_addresses": ["0x9459c8cb7424a2946e6bcf7bc204e349a2865f84f2ae75586ada2897d74c4e"],
            "contracts_storage_keys": [ {
                "contract_address": "0x49d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7",
                "storage_keys": ["0x5d2e9527cbeb1a51aa084b0de7501f343b7b1bf24a0c427d6204a7b7988970", "0x1390569bb0a3a722eb4228e8700301347da081211d5c2ded2db22ef389551ab"]
            } ]
        },
        "id": 1
    })).send().await.unwrap().json().await.unwrap();

    let mut expected = serde_json::json!({
      "classes_proof": [],
      "contracts_proof": {
        "contract_leaves_data": [
          {
            "class_hash": "0x348f560344334951bcbccd27aff05a9f6bedeaefc36315fe20e842163adae3d",
            "nonce": "0x0"
          }
        ],
        "nodes": [
          {
            "node": {
              "child": "0x3873db7e0ca5c27a75e9e867f61124c2df15095124b413cccf94e1f50427d1f",
              "length": 247,
              "path": "0x1459c8cb7424a2946e6bcf7bc204e349a2865f84f2ae75586ada2897d74c4e0"
            },
            "node_hash": "0x556551ef2d003cf08323a6c1c60d8de89ec9e6450700b62d0a4e81378a0658d"
          },
          {
            "node": {
              "left": "0x113a4cb02d845bd88a5466435c9288da65599ff63489cafbe7b187a355d6af5",
              "right": "0x556551ef2d003cf08323a6c1c60d8de89ec9e6450700b62d0a4e81378a0658d"
            },
            "node_hash": "0x7e0158524ad463e162a32a538d652d9be1d05a4248e9273f6c4edf6bbe9c031"
          },
          {
            "node": {
              "left": "0x3476bae64cf909357043bafba4ac368317c39930dd8d73a753f73e7cd012425",
              "right": "0x2957bdb91e01a0a1524ca6751d0b302374f0976cf5268c9e765dd878479644b"
            },
            "node_hash": "0x75ed61949488f1fb243997925cd7717365673860adcbae04115655daa5a46a4"
          },
          {
            "node": {
              "left": "0x777087b1140cf36e78f089842b5cbe8f5d1e58d5517dcc70a01d02ea72b2ad1",
              "right": "0x6be4e94f1013c755aea91f4fca72b796777991bfe97ed4fe02924bcaa72f2eb"
            },
            "node_hash": "0x3476bae64cf909357043bafba4ac368317c39930dd8d73a753f73e7cd012425"
          },
          {
            "node": {
              "left": "0x7e0158524ad463e162a32a538d652d9be1d05a4248e9273f6c4edf6bbe9c031",
              "right": "0x671471198ae4ebf7758c8e3a74fd22f80d3407ca298036b2f2938342168269e"
            },
            "node_hash": "0x777087b1140cf36e78f089842b5cbe8f5d1e58d5517dcc70a01d02ea72b2ad1"
          }
        ]
      },
      "contracts_storage_proofs": [
        [
          {
            "node": {
              "child": "0x23be95f90bf41685e18a4356e57b0cfdc1da22bf382ead8b64108353915c1e5",
              "length": 247,
              "path": "0x5d2e9527cbeb1a51aa084b0de7501f343b7b1bf24a0c427d6204a7b79889700"
            },
            "node_hash": "0x6da041953eaf48fefeab68c933739b8b69fd9f98c05d44c83f3a495963f322b"
          },
          {
            "node": {
              "child": "0x3029e51ebd2307c7e813f71c30653ba30444f2722b300dbe52970910c7a81b3",
              "length": 1,
              "path": "0x0"
            },
            "node_hash": "0x1d50341c7240897d3733e6159f821adb69197bffea380b9298b34a0343d8f"
          },
          {
            "node": {
              "child": "0x4c5772d1914fe6ce891b64eb35bf3522aeae1315647314aac58b01137607f3f",
              "length": 245,
              "path": "0x6415a6ec28e9c8bad08a3a1c00c04d1f68204847570b7b4b6c8bbce25546ac0"
            },
            "node_hash": "0x3b8aa8795ea2891d6ee1086fdfd85ddd88be6bc7aa1fb0b4f9b9357bfa9b686"
          },
          {
            "node": {
              "left": "0x1d50341c7240897d3733e6159f821adb69197bffea380b9298b34a0343d8f",
              "right": "0x88b1b3f0534367fc6420c0f3f2e18342456b5f36c3eba15aeec2c60176dde0"
            },
            "node_hash": "0x92c9f66f32ccf384cd5a5888e0dc4419952ff0152e4969f64c4d434d7d579d"
          },
          {
            "node": {
              "left": "0x285bc4342724dd9d19991811fcf952273723569ddc45925592b8939c7849d8a",
              "right": "0x5f077f4551dcc7afdc85e0bc55d23c5c823886a68361be55019aba06f81504e"
            },
            "node_hash": "0x1d84ed85ea21afefe5c8f8529f2839ceb2713b7c54d09092548acf2a358adc4"
          },
          {
            "node": {
              "left": "0x3c1c6fc0a86454b3840874bbe4ea4911f5f9e2ce5179f44f0ff4faafa76f786",
              "right": "0x77fc03894c569b6036a7f5f8989f726b6f96c95f89ce3a0773ad2235dc8eec9"
            },
            "node_hash": "0x285bc4342724dd9d19991811fcf952273723569ddc45925592b8939c7849d8a"
          },
          {
            "node": {
              "left": "0x49a24000932a1274b2ef7cafe894080354b0d6de72af76b1b46ad3fe4dff8f0",
              "right": "0x92c9f66f32ccf384cd5a5888e0dc4419952ff0152e4969f64c4d434d7d579d"
            },
            "node_hash": "0x3c1c6fc0a86454b3840874bbe4ea4911f5f9e2ce5179f44f0ff4faafa76f786"
          },
          {
            "node": {
              "left": "0x6da041953eaf48fefeab68c933739b8b69fd9f98c05d44c83f3a495963f322b",
              "right": "0x71d557b3e9306280428f9fa5ed9d1c04508b037613a8b1e8dd805290615e33"
            },
            "node_hash": "0x49a24000932a1274b2ef7cafe894080354b0d6de72af76b1b46ad3fe4dff8f0"
          },
          {
            "node": {
              "left": "0x74bd3ad6b95f9a1333986d481392c536ea7856b0aaf5a8d97abc43557c48ee3",
              "right": "0x3b8aa8795ea2891d6ee1086fdfd85ddd88be6bc7aa1fb0b4f9b9357bfa9b686"
            },
            "node_hash": "0x3029e51ebd2307c7e813f71c30653ba30444f2722b300dbe52970910c7a81b3"
          }
        ]
      ],
      "global_roots": {
        "block_hash": "0x5beb56c7d9a9fc066e695c3fc467f45532cace83d9979db4ccfd6b77ca476af",
        "classes_tree_root": "0x5ff17fbd8a130acb96f6c97f9f926a2acdc59a76c34aba6ec160f74e30e6635",
        "contracts_tree_root": "0x75ed61949488f1fb243997925cd7717365673860adcbae04115655daa5a46a4"
      }
    });

    normalize(&mut ret["result"]);
    normalize(&mut expected);
    assert_eq!(ret["result"], expected);

    // non membership proof, and more in the past.
    let mut ret: serde_json::Value = client
        .post(node.rpc_url.clone().unwrap())
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "method": "starknet_getStorageProof",
            "params": {
                "block_id": { "block_number": 5 },
                "contract_addresses": ["0x9459c8cb7424a2946e6bcf7bc204e349a2865f000000000000000897d74c4e"]
            },
            "id": 1
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let mut expected = serde_json::json!({
      "classes_proof": [],
      "contracts_proof": {
        "contract_leaves_data": [
          {
            "class_hash": "0x0",
            "nonce": "0x0"
          }
        ],
        "nodes": [
          {
            "node": {
              "child": "0x3873db7e0ca5c27a75e9e867f61124c2df15095124b413cccf94e1f50427d1f",
              "length": 247,
              "path": "0x1459c8cb7424a2946e6bcf7bc204e349a2865f84f2ae75586ada2897d74c4e0"
            },
            "node_hash": "0x556551ef2d003cf08323a6c1c60d8de89ec9e6450700b62d0a4e81378a0658d"
          },
          {
            "node": {
              "left": "0x113a4cb02d845bd88a5466435c9288da65599ff63489cafbe7b187a355d6af5",
              "right": "0x556551ef2d003cf08323a6c1c60d8de89ec9e6450700b62d0a4e81378a0658d"
            },
            "node_hash": "0x7e0158524ad463e162a32a538d652d9be1d05a4248e9273f6c4edf6bbe9c031"
          },
          {
            "node": {
              "left": "0x3476bae64cf909357043bafba4ac368317c39930dd8d73a753f73e7cd012425",
              "right": "0x2957bdb91e01a0a1524ca6751d0b302374f0976cf5268c9e765dd878479644b"
            },
            "node_hash": "0x75ed61949488f1fb243997925cd7717365673860adcbae04115655daa5a46a4"
          },
          {
            "node": {
              "left": "0x777087b1140cf36e78f089842b5cbe8f5d1e58d5517dcc70a01d02ea72b2ad1",
              "right": "0x6be4e94f1013c755aea91f4fca72b796777991bfe97ed4fe02924bcaa72f2eb"
            },
            "node_hash": "0x3476bae64cf909357043bafba4ac368317c39930dd8d73a753f73e7cd012425"
          },
          {
            "node": {
              "left": "0x7e0158524ad463e162a32a538d652d9be1d05a4248e9273f6c4edf6bbe9c031",
              "right": "0x671471198ae4ebf7758c8e3a74fd22f80d3407ca298036b2f2938342168269e"
            },
            "node_hash": "0x777087b1140cf36e78f089842b5cbe8f5d1e58d5517dcc70a01d02ea72b2ad1"
          }
        ]
      },
      "contracts_storage_proofs": [],
      "global_roots": {
        "block_hash": "0x13b390a0b2c48f907cda28c73a12aa31b96d51bc1be004ba5f71174d8d70e4f",
        "classes_tree_root": "0x5ff17fbd8a130acb96f6c97f9f926a2acdc59a76c34aba6ec160f74e30e6635",
        "contracts_tree_root": "0x75ed61949488f1fb243997925cd7717365673860adcbae04115655daa5a46a4"
      }
    });

    normalize(&mut ret["result"]);
    normalize(&mut expected);
    println!("{}", serde_json::to_string_pretty(&ret["result"]).unwrap());
    assert_eq!(ret["result"], expected);
}
