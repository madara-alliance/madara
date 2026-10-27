//! Opt-in signed private-RPC dispatch measurement, including JSON decoding and durable ACK.
use super::*;
use blockifier::execution::syscalls::committed_data::{CommittedDataBatchProof, CommittedDataSet};
use mc_db::MadaraStorageWrite;
use std::time::Instant;

mod concurrent;

fn parameter(name: &str, default: usize) -> usize {
    std::env::var(name).map(|s| s.parse().expect("integer benchmark parameter")).unwrap_or(default)
}
fn random(key: usize, tick: usize) -> Felt {
    starknet_core::utils::starknet_keccak(format!("rpc-price-{key}-{tick}").as_bytes())
}
fn changed_price(key: usize, tick: usize, previous: Option<Felt>, large_moves: bool) -> Felt {
    let random = u64::from_be_bytes(random(key, tick).to_bytes_be()[24..].try_into().unwrap());
    let value = Felt::from(match previous.filter(|_| !large_moves) {
        Some(old) => {
            let old = u64::from_be_bytes(old.to_bytes_be()[24..].try_into().unwrap());
            let change = 1 + random % (old / 65536).max(1);
            if random & 1 == 0 {
                old + change
            } else {
                old.saturating_sub(change).max(1)
            }
        }
        None => {
            let bits = [23, 27, 29, 30, 32, 33, 35, 36, 38, 41][key % 10];
            (1 << (bits - 1)) | (random & ((1 << (bits - 1)) - 1))
        }
    });
    if Some(value) == previous {
        value + Felt::ONE
    } else {
        value
    }
}

#[test]
fn committed_data_rpc_price_redraw_collision_still_changes_value() {
    let drawn = changed_price(0, 1, None, true);
    assert_eq!(changed_price(0, 1, Some(drawn), true), drawn + Felt::ONE);
}

#[tokio::test]
#[ignore = "Opt-in signed 50k/100k import measurement on the actual RPC/backend"]
async fn committed_data_measure_signed_rpc_imports() {
    let entries = parameter("CD_MEASURE_ENTRIES", 50000);
    let versions = parameter("CD_MEASURE_VERSIONS", 16);
    let retained = parameter("CD_MEASURE_RETAINED", 8);
    let large_moves = std::env::var("CD_MEASURE_LARGE_MOVES").is_ok_and(|v| v == "true");
    assert!((1..=524288).contains(&entries) && versions <= 1000 && versions > retained && retained > 0);
    let (backend, mut rpc) = crate::test_utils::rpc_test_setup();
    let public_key = starknet_types_core::curve::AffinePoint::generator().x();
    rpc.set_committed_data_signers(vec![public_key]).unwrap();
    let module = crate::rpc_api_committed_data(&rpc).unwrap();
    let mut values: Vec<_> =
        (0..entries).map(|key| (random(key, usize::MAX), changed_price(key, 0, None, false))).collect();
    let mut base_root = None;
    let chain = backend.chain_config().chain_id.to_felt();
    let mut timings = Vec::new();
    let mut cycles = Vec::new();
    let mut first = None;
    let keys: Vec<_> = values.iter().take(32).map(|(key, _)| *key).collect();
    for tick in 0..=versions {
        if tick > 0 {
            for (key, (_, price)) in values.iter_mut().enumerate() {
                let old = *price;
                *price = changed_price(key, tick, Some(old), large_moves);
                assert_ne!(*price, old);
            }
        }
        let tree = CommittedDataSet::new(values.clone()).unwrap(); // Client work, outside server timer.
        let root = tree.root();
        if tick == 0 {
            first = Some((root, CommittedDataBatchProof::from_dataset(&tree, &keys).unwrap()));
        }
        let update = CommittedDataUpdate { application_id: Felt::ONE, dataset_id: Felt::TWO, base_root };
        let signature =
            starknet_core::crypto::ecdsa_sign(&Felt::ONE, &update_message_hash(chain, update, root, entries as u32))
                .unwrap();
        let request = serde_json::json!({"jsonrpc":"2.0","id":tick,
            "method":"madara_V0_1_0_updateCommittedDataSet",
            "params":[update, root, values, [ImportSignature { public_key, r: signature.r, s: signature.s }]]})
        .to_string();
        let started = Instant::now();
        let (response, _) = module.raw_json_request(&request, 1).await.unwrap();
        timings.push(started.elapsed().as_secs_f64() * 1000.);
        let response: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert!(response.get("error").is_none(), "{response}");
        assert_eq!(response.get("result"), Some(&serde_json::Value::Null));
        let journal = backend.recovery_reference(Some(update), root).unwrap();
        // Simulated asynchronous export completion.
        backend.db.mark_recovery_exported(&journal).unwrap();
        // Record the simulated execution height before allowing retention to reclaim old roots.
        backend.db.note_committed_data_use(root, tick as u64 + 1).unwrap();
        if tick >= retained {
            while backend.db.maintain_committed_data_history(Some((tick + 2 - retained) as u64)).unwrap() {}
        }
        cycles.push(started.elapsed().as_secs_f64() * 1000.);
        base_root = Some(root);
    }
    let (root, proof) = first.unwrap();
    assert_eq!(backend.db.get_committed_data_batch(root, &keys).unwrap(), proof);
    let report = serde_json::json!({"entries": entries, "versions": versions + 1, "large_moves": large_moves,
        "initial_signed_rpc_ms": timings[0], "signed_rpc_ms": &timings[1..], "cycle_including_pruning_ms": &cycles[1..],
        "all_prices_changed": true, "historical_proof_matches": true,
        "limitations": "In-process raw JSON dispatch with real signature validation and RocksDB WAL sync; excludes network transport, account transaction submission and actual Mongo export. Client root/signature construction is outside timing. Synthetic changing prices, not captured transactions."});
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
    if let Ok(path) = std::env::var("CD_MEASURE_RPC_REPORT") {
        std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
}
