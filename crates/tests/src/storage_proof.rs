use crate::MadaraCmdBuilder;
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
    let mut ret: serde_json::Value = client.post(node.rpc_url.clone()).json(&serde_json::json!({
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
                        "left": "0x3e24ca8795469f6af2da26e75b8e764dd85ad6d14f0f64f522b21aec2ff78ba",
                        "right": "0x17b6653fa9473af8b2c74d5dd413e0905e71c48568c570fff8150110d54106d"
                    },
                    "node_hash": "0x34d7f08975375c6f5b5aa183e047c445a323615d39f29b6880e6fb590e914ad"
                },
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
                        "left": "0x34d7f08975375c6f5b5aa183e047c445a323615d39f29b6880e6fb590e914ad",
                        "right": "0x6be4e94f1013c755aea91f4fca72b796777991bfe97ed4fe02924bcaa72f2eb"
                    },
                    "node_hash": "0x46c84b36c0e47eb84d10791590ad616d06267ae8c33008ede4d34bba8e0c4f5"
                },
                {
                    "node": {
                        "left": "0x46c84b36c0e47eb84d10791590ad616d06267ae8c33008ede4d34bba8e0c4f5",
                        "right": "0xfb52642774e8f691b47c7c0ba2af7b2e205bbc10cd31da872cbf214ada6b75"
                    },
                    "node_hash": "0x4196effc79506ae6622b1e38d10dc9d42debf149dd63879b1052db1fd191664"
                },
                {
                    "node": {
                        "left": "0x1c5a6c9966c9f7a1f8f60f2d50cb63fcfa5b261d4549187ac274ab4272cc5e3",
                        "right": "0x556551ef2d003cf08323a6c1c60d8de89ec9e6450700b62d0a4e81378a0658d"
                    },
                    "node_hash": "0x3e24ca8795469f6af2da26e75b8e764dd85ad6d14f0f64f522b21aec2ff78ba"
                }
            ]
        },
        "contracts_storage_proofs": [
            [
                {
                    "node": {
                        "left": "0x3c1c6fc0a86454b3840874bbe4ea4911f5f9e2ce5179f44f0ff4faafa76f786",
                        "right": "0x845b0b9670e0ae63d86c25df621c46524d8b5aca4461bf73ad5e991065230"
                    },
                    "node_hash": "0xfbeccd24c1ec3d210e2935bb9058fa04e279b75d5a329dbcd641ad679eb2c7"
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
                        "left": "0xfbeccd24c1ec3d210e2935bb9058fa04e279b75d5a329dbcd641ad679eb2c7",
                        "right": "0x74702711d4d0c46c1cb57aa4814f8ebd586ecd0ecad9f2e85c9d3c5d544de43"
                    },
                    "node_hash": "0x4f10a91055749d100e2833fa26627be2b1cc15c118b36bac21d44b6e23fee4f"
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
                        "left": "0x6da041953eaf48fefeab68c933739b8b69fd9f98c05d44c83f3a495963f322b",
                        "right": "0x71d557b3e9306280428f9fa5ed9d1c04508b037613a8b1e8dd805290615e33"
                    },
                    "node_hash": "0x49a24000932a1274b2ef7cafe894080354b0d6de72af76b1b46ad3fe4dff8f0"
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
                        "child": "0x23be95f90bf41685e18a4356e57b0cfdc1da22bf382ead8b64108353915c1e5",
                        "length": 247,
                        "path": "0x5d2e9527cbeb1a51aa084b0de7501f343b7b1bf24a0c427d6204a7b79889700"
                    },
                    "node_hash": "0x6da041953eaf48fefeab68c933739b8b69fd9f98c05d44c83f3a495963f322b"
                },
                {
                    "node": {
                        "left": "0x74bd3ad6b95f9a1333986d481392c536ea7856b0aaf5a8d97abc43557c48ee3",
                        "right": "0x3b8aa8795ea2891d6ee1086fdfd85ddd88be6bc7aa1fb0b4f9b9357bfa9b686"
                    },
                    "node_hash": "0x3029e51ebd2307c7e813f71c30653ba30444f2722b300dbe52970910c7a81b3"
                },
                {
                    "node": {
                        "child": "0x4c5772d1914fe6ce891b64eb35bf3522aeae1315647314aac58b01137607f3f",
                        "length": 245,
                        "path": "0x6415a6ec28e9c8bad08a3a1c00c04d1f68204847570b7b4b6c8bbce25546ac0"
                    },
                    "node_hash": "0x3b8aa8795ea2891d6ee1086fdfd85ddd88be6bc7aa1fb0b4f9b9357bfa9b686"
                }
            ]
        ],
        "global_roots": {
            "block_hash": "0x4177d1ba942a4ab94f86a476c06f0f9e02363ad410cdf177c54064788c9bcb5",
            "classes_tree_root": "0x6eeb97e12755a43e64901f54239ebcb0a0f9b2b87fca9b91dd7059689df7a9",
            "contracts_tree_root": "0x4196effc79506ae6622b1e38d10dc9d42debf149dd63879b1052db1fd191664"
        }
    });

    normalize(&mut ret["result"]);
    normalize(&mut expected);
    assert_eq!(ret["result"], expected);

    // non membership proof, and in the past.
    let mut ret: serde_json::Value = client
        .post(node.rpc_url.clone())
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
                        "child": "0x4e535350f60feff5eca7ee0819d2c86029c4e987ee800c8b9fabdb0167d17ae",
                        "length": 250,
                        "path": "0x477d2bf217e82d0bc31486adcaf619fb83b4457e705d5b16c82106a722b83ca"
                    },
                    "node_hash": "0x7cec9cc48ff8fdf1a4f9d5235a6cae25222b6bb824f8ab3bc35f57c82e856a5"
                },
                {
                    "node": {
                        "left": "0x7cec9cc48ff8fdf1a4f9d5235a6cae25222b6bb824f8ab3bc35f57c82e856a5",
                        "right": "0x17f07e9ead8e5a5c085a17e6b41e5d565bac7433095a91ded8c007c1bf68e90"
                    },
                    "node_hash": "0xe290a369aa37fb11008ffea52cf207cd5e2a66fed9ad4e551aa46ec0ed58b"
                }
            ]
        },
        "contracts_storage_proofs": [],
        "global_roots": {
            "block_hash": "0x13b390a0b2c48f907cda28c73a12aa31b96d51bc1be004ba5f71174d8d70e4f",
            "classes_tree_root": "0x5ff17fbd8a130acb96f6c97f9f926a2acdc59a76c34aba6ec160f74e30e6635",
            "contracts_tree_root": "0xe290a369aa37fb11008ffea52cf207cd5e2a66fed9ad4e551aa46ec0ed58b"
        }
    });

    normalize(&mut ret["result"]);
    normalize(&mut expected);
    assert_eq!(ret["result"], expected);
}
