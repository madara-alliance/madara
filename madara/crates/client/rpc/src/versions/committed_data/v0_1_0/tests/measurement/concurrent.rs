//! Loopback HTTP response timing while a real, finite proof-index backlog is reclaimed.
use super::*;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

fn memory_kib(field: &str) -> Option<u64> {
    std::fs::read_to_string("/proc/self/status").ok()?.lines().find_map(|line| {
        let mut words = line.split_whitespace();
        (words.next()? == field).then(|| words.next()?.parse().ok()).flatten()
    })
}

fn signed_request(chain: Felt, update: CommittedDataUpdate, root: Felt, values: &[(Felt, Felt)]) -> String {
    let signature =
        starknet_core::crypto::ecdsa_sign(&Felt::ONE, &update_message_hash(chain, update, root, values.len() as u32))
            .unwrap();
    let public_key = starknet_types_core::curve::AffinePoint::generator().x();
    serde_json::json!({"jsonrpc":"2.0","id":1,"method":"madara_V0_1_0_updateCommittedDataSet",
        "params":[update, root, values, [ImportSignature { public_key, r: signature.r, s: signature.s }]]})
    .to_string()
}

fn assert_success(response: &str) {
    let response: serde_json::Value = serde_json::from_str(response).unwrap();
    assert!(response.get("error").is_none(), "{response}");
    assert_eq!(response.get("result"), Some(&serde_json::Value::Null));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "Opt-in HTTP import latency/RSS under concurrent real proof pruning"]
async fn committed_data_measure_concurrent_http_imports() {
    let entries = parameter("CD_MEASURE_ENTRIES", 50000);
    let versions = parameter("CD_MEASURE_VERSIONS", 32);
    let large_moves = std::env::var("CD_MEASURE_LARGE_MOVES").is_ok_and(|v| v == "true");
    let warmup = 128usize;
    assert!((1..=524288).contains(&entries) && (16..=128).contains(&versions));
    let (backend, mut rpc) = crate::test_utils::rpc_test_setup();
    rpc.set_committed_data_signers(vec![starknet_types_core::curve::AffinePoint::generator().x()]).unwrap();
    let module = crate::rpc_api_committed_data(&rpc).unwrap();
    let server = jsonrpsee::server::Server::builder()
        .max_request_body_size(128 * 1024 * 1024)
        .build("127.0.0.1:0")
        .await
        .unwrap();
    let url = format!("http://{}", server.local_addr().unwrap());
    let handle = server.start(module);
    let client = reqwest::Client::builder().timeout(Duration::from_secs(60)).build().unwrap();
    let mut values: Vec<_> =
        (0..entries).map(|key| (random(key, usize::MAX), changed_price(key, 0, None, false))).collect();
    let chain = backend.chain_config().chain_id.to_felt();
    let keys: Vec<_> = values.iter().take(32).map(|(key, _)| *key).collect();
    let mut base_root = None;
    let mut first = None;
    let origin = Instant::now();
    let mut samples = Vec::new();
    let mut warmup_timings = Vec::new();
    let stop = Arc::new(AtomicBool::new(false));
    let mut pruner = None;
    let mut peak_active_rss = 0;
    for tick in 0..=warmup + versions {
        if tick > 0 {
            for (key, (_, price)) in values.iter_mut().enumerate() {
                let old = *price;
                *price = changed_price(key, tick, Some(old), large_moves);
                assert_ne!(*price, old);
            }
        }
        // Publisher work is outside response timing, but remains part of process RSS/CPU.
        let tree = CommittedDataSet::new(values.clone()).unwrap();
        let root = tree.root();
        if first.is_none() {
            first = Some((root, CommittedDataBatchProof::from_dataset(&tree, &keys).unwrap()));
        }
        drop(tree);
        let update = CommittedDataUpdate { application_id: Felt::ONE, dataset_id: Felt::TWO, base_root };
        let request = signed_request(chain, update, root, &values);
        if tick == warmup + 1 {
            let backend = Arc::clone(&backend);
            let stop = Arc::clone(&stop);
            // This is the service's one-root-at-a-time loop during an eligible sweep.
            // Freeze the simulated settlement cutoff so the current import base remains hot.
            pruner = Some(std::thread::spawn(move || {
                let mut intervals = Vec::new();
                while !stop.load(Ordering::Relaxed) {
                    let start = origin.elapsed().as_secs_f64() * 1000.;
                    if !backend.db.maintain_committed_data_history(Some((warmup - 8) as u64)).unwrap() {
                        break;
                    }
                    intervals.push((start, origin.elapsed().as_secs_f64() * 1000.));
                }
                intervals
            }));
        }
        let start = origin.elapsed().as_secs_f64() * 1000.;
        let response = client.post(&url).header("content-type", "application/json").body(request).send();
        // Sample the active phase while awaiting the complete HTTP response body.
        let response = async { response.await.unwrap().error_for_status().unwrap().text().await.unwrap() };
        tokio::pin!(response);
        let response = loop {
            tokio::select! {
                response = &mut response => break response,
                _ = tokio::time::sleep(Duration::from_millis(20)), if tick > warmup => {
                    peak_active_rss = peak_active_rss.max(memory_kib("VmRSS:").unwrap_or(0));
                }
            }
        };
        let end = origin.elapsed().as_secs_f64() * 1000.;
        assert_success(&response);
        if tick > warmup {
            samples.push((start, end));
        } else {
            warmup_timings.push(end - start);
        }
        let journal = backend.recovery_reference(Some(update), root).unwrap();
        backend.db.mark_recovery_exported(&journal).unwrap(); // Simulated external exporter ACK.
        backend.db.note_committed_data_use(root, tick as u64 + 1).unwrap();
        base_root = Some(root);
    }
    stop.store(true, Ordering::Relaxed);
    let intervals = tokio::task::spawn_blocking(move || pruner.unwrap().join().unwrap()).await.unwrap();
    let (root, proof) = first.unwrap();
    assert_eq!(backend.db.get_committed_data_batch(root, &keys).unwrap(), proof);
    let latest = CommittedDataSet::new(values).unwrap();
    assert_eq!(
        backend.db.get_committed_data_batch(latest.root(), &keys).unwrap(),
        CommittedDataBatchProof::from_dataset(&latest, &keys).unwrap()
    );
    handle.stop().unwrap();
    handle.stopped().await;
    let timings: Vec<_> = samples.iter().map(|(start, end)| end - start).collect();
    let overlapping: Vec<_> = samples
        .iter()
        .enumerate()
        .filter(|(_, (start, end))| intervals.iter().any(|(ps, pe)| ps < end && pe > start))
        .map(|(index, _)| index)
        .collect();
    let report = serde_json::json!({"entries":entries,"warmup_versions":warmup+1,"measured_requests":versions,
        "large_moves":large_moves,"all_prices_changed":true,"http_response_ms":timings,
        "warmup_http_response_ms":warmup_timings,
        "request_intervals_ms":samples,"prune_intervals_ms":intervals,"overlapping_request_indices":overlapping,
        "peak_active_rss_kib":peak_active_rss,"whole_test_peak_rss_kib":memory_kib("VmHWM:"),
        "historical_and_latest_proofs_match":true,
        "limitations":"Loopback HTTP, real signatures and durable RocksDB ACK. Synthetic all-changing prices; publisher construction shares process CPU/RAM outside response timers. Finite 129-version backlog, not six hours. Simulated settlement eligibility and Mongo export acknowledgements; no account transaction submission. Pruning intervals include time waiting for the shared lock."});
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
    if let Ok(path) = std::env::var("CD_MEASURE_RPC_REPORT") {
        std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
    assert!(overlapping.len() >= 16, "Need at least sixteen HTTP requests overlapping actual pruning work");
    assert!(
        timings.iter().chain(&warmup_timings).all(|ms| *ms < 5000.),
        "HTTP import response exceeded five seconds; see report"
    );
}
