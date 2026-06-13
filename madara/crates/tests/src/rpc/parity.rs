#[cfg(test)]
mod test_rpc_parity {
    use crate::MadaraCmdBuilder;
    use serde::{Deserialize, Serialize};
    use serde_json::{json, Value};
    use std::{
        collections::BTreeSet,
        env, fs,
        path::{Path, PathBuf},
    };

    const PARITY_VERSION: &str = "v0_10_3";
    const PARITY_ROUTE: &str = "/rpc/v0_10_3/";
    const ETH_TOKEN_ADDRESS: &str = "0x49d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7";
    const ETH_NAME_STORAGE_KEY: &str = "0x0341c1bdfd89f69748aa00b5742b03adbffd79b8e80cab5c50d91cd8c2a79be1";
    const DEFAULT_TX: &str = "0x701d9adb9c60bc2fd837fe3989e15aeba4be1a6e72bb6f61ffe35a42866c772";

    #[derive(Debug, Deserialize)]
    struct AllowlistEntry {
        provider: String,
        version: String,
        method: String,
        path: String,
    }

    #[derive(Debug, Serialize)]
    struct DiffReport {
        provider: String,
        version: String,
        method: String,
        params: Value,
        diffs: Vec<JsonDiff>,
    }

    #[derive(Debug, Clone, Serialize)]
    struct JsonDiff {
        path: String,
        madara: Value,
        reference: Value,
    }

    #[derive(Debug, Clone)]
    struct RpcCase {
        method: String,
        params: Value,
    }

    #[derive(Debug)]
    struct Provider {
        name: &'static str,
        url: String,
    }

    fn required_env(name: &str) -> Option<String> {
        match env::var(name) {
            Ok(value) if !value.trim().is_empty() => Some(value),
            _ => None,
        }
    }

    fn parse_csv_u64(name: &str, default: &[u64]) -> Vec<u64> {
        let Ok(raw) = env::var(name) else { return default.to_vec() };
        let values = raw
            .split(',')
            .filter_map(|value| {
                let value = value.trim();
                (!value.is_empty()).then(|| value.parse::<u64>().expect("invalid MADARA_RPC_PARITY_BLOCKS value"))
            })
            .collect::<Vec<_>>();
        if values.is_empty() {
            default.to_vec()
        } else {
            values
        }
    }

    fn parse_csv_strings(name: &str, default: &[&str]) -> Vec<String> {
        let Ok(raw) = env::var(name) else { return default.iter().map(|value| value.to_string()).collect() };
        let values: Vec<String> =
            raw.split(',').map(str::trim).filter(|value| !value.is_empty()).map(str::to_owned).collect();
        if values.is_empty() {
            default.iter().map(|value| value.to_string()).collect()
        } else {
            values
        }
    }

    fn report_dir() -> PathBuf {
        env::var("MADARA_RPC_PARITY_REPORT_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../target/rpc-parity"))
    }

    fn allowlist() -> Vec<AllowlistEntry> {
        serde_json::from_str(include_str!("../../fixtures/rpc-parity-allowlist.json"))
            .expect("RPC parity allowlist should parse")
    }

    fn is_allowed(allowlist: &[AllowlistEntry], provider: &str, method: &str, path: &str) -> bool {
        allowlist.iter().any(|entry| {
            (entry.provider == provider || entry.provider == "*")
                && (entry.version == PARITY_VERSION || entry.version == "*")
                && (entry.method == method || entry.method == "*")
                && (entry.path == path || entry.path == "*")
        })
    }

    async fn rpc_response(url: &str, method: &str, params: Value) -> anyhow::Result<Value> {
        let timeout_secs =
            env::var("MADARA_RPC_PARITY_TIMEOUT_SECS").ok().and_then(|value| value.parse().ok()).unwrap_or(30);
        let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(timeout_secs)).build()?;
        let payload = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
            "id": 1
        });
        let response = client.post(url).json(&payload).send().await?;
        anyhow::ensure!(response.status().is_success(), "{method} returned HTTP {}", response.status());
        Ok(response.json().await?)
    }

    fn normalize_response(mut value: Value) -> Value {
        if let Some(object) = value.as_object_mut() {
            object.remove("id");
            object.remove("jsonrpc");
        }
        value
    }

    fn json_path_for_key(parent: &str, key: &str) -> String {
        if key.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_') {
            format!("{parent}.{key}")
        } else {
            format!("{parent}[{}]", serde_json::to_string(key).expect("serializing key"))
        }
    }

    fn diff_values(path: &str, madara: &Value, reference: &Value, diffs: &mut Vec<JsonDiff>) {
        match (madara, reference) {
            (Value::Object(left), Value::Object(right)) => {
                let keys = left.keys().chain(right.keys()).cloned().collect::<BTreeSet<_>>();
                for key in keys {
                    let next_path = json_path_for_key(path, &key);
                    match (left.get(&key), right.get(&key)) {
                        (Some(left), Some(right)) => diff_values(&next_path, left, right, diffs),
                        (Some(left), None) => {
                            diffs.push(JsonDiff { path: next_path, madara: left.clone(), reference: Value::Null })
                        }
                        (None, Some(right)) => {
                            diffs.push(JsonDiff { path: next_path, madara: Value::Null, reference: right.clone() })
                        }
                        (None, None) => {}
                    }
                }
            }
            (Value::Array(left), Value::Array(right)) => {
                let max_len = left.len().max(right.len());
                for index in 0..max_len {
                    let next_path = format!("{path}[{index}]");
                    match (left.get(index), right.get(index)) {
                        (Some(left), Some(right)) => diff_values(&next_path, left, right, diffs),
                        (Some(left), None) => {
                            diffs.push(JsonDiff { path: next_path, madara: left.clone(), reference: Value::Null })
                        }
                        (None, Some(right)) => {
                            diffs.push(JsonDiff { path: next_path, madara: Value::Null, reference: right.clone() })
                        }
                        (None, None) => {}
                    }
                }
            }
            _ if madara == reference => {}
            _ => diffs.push(JsonDiff { path: path.to_owned(), madara: madara.clone(), reference: reference.clone() }),
        }
    }

    fn parity_cases(blocks: &[u64], txs: &[String]) -> Vec<RpcCase> {
        let mut cases = vec![
            RpcCase { method: "starknet_blockHashAndNumber".to_string(), params: json!({}) },
            RpcCase {
                method: "starknet_getEvents".to_string(),
                params: json!({
                    "filter": {
                        "from_block": {"block_number": blocks[0]},
                        "to_block": {"block_number": *blocks.last().expect("blocks should not be empty")},
                        "keys": [[]],
                        "chunk_size": 10
                    }
                }),
            },
            RpcCase {
                method: "starknet_getStorageAt".to_string(),
                params: json!({
                    "contract_address": ETH_TOKEN_ADDRESS,
                    "key": ETH_NAME_STORAGE_KEY,
                    "block_id": {"block_number": *blocks.last().expect("blocks should not be empty")}
                }),
            },
            RpcCase {
                method: "starknet_getStorageProof".to_string(),
                params: json!({
                    "block_id": {"block_number": *blocks.last().expect("blocks should not be empty")},
                    "class_hashes": [],
                    "contract_addresses": [],
                    "contracts_storage_keys": []
                }),
            },
            RpcCase {
                method: "starknet_traceBlockTransactions".to_string(),
                params: json!({"block_id": {"block_number": *blocks.last().expect("blocks should not be empty")}}),
            },
            RpcCase {
                method: "starknet_traceBlockTransactions".to_string(),
                params: json!({
                    "block_id": {"block_number": *blocks.last().expect("blocks should not be empty")},
                    "trace_flags": ["RETURN_INITIAL_READS"]
                }),
            },
            RpcCase { method: "starknet_getMessagesStatus".to_string(), params: json!({"transaction_hash": "0x1"}) },
        ];

        for block in blocks {
            let block_id = json!({"block_number": block});
            cases.extend([
                RpcCase { method: "starknet_getBlockWithTxHashes".to_string(), params: json!({"block_id": block_id}) },
                RpcCase {
                    method: "starknet_getBlockWithTxs".to_string(),
                    params: json!({"block_id": {"block_number": block}}),
                },
                RpcCase {
                    method: "starknet_getBlockWithReceipts".to_string(),
                    params: json!({"block_id": {"block_number": block}}),
                },
                RpcCase {
                    method: "starknet_getBlockTransactionCount".to_string(),
                    params: json!({"block_id": {"block_number": block}}),
                },
            ]);
        }

        for tx in txs {
            cases.extend([
                RpcCase {
                    method: "starknet_getTransactionByHash".to_string(),
                    params: json!({"transaction_hash": tx}),
                },
                RpcCase {
                    method: "starknet_getTransactionReceipt".to_string(),
                    params: json!({"transaction_hash": tx}),
                },
                RpcCase {
                    method: "starknet_getTransactionStatus".to_string(),
                    params: json!({"transaction_hash": tx}),
                },
                RpcCase { method: "starknet_traceTransaction".to_string(), params: json!({"transaction_hash": tx}) },
            ]);
        }

        cases
    }

    fn write_report(path: &Path, reports: &[DiffReport]) -> anyhow::Result<()> {
        fs::create_dir_all(path)?;
        let report_path = path.join("rpc-parity-diff-report.json");
        fs::write(report_path, serde_json::to_vec_pretty(reports)?)?;
        Ok(())
    }

    #[tokio::test]
    async fn rpc_parity_against_configured_reference_nodes() -> anyhow::Result<()> {
        let Some(juno_url) = required_env("MADARA_RPC_PARITY_JUNO_URL") else {
            eprintln!("skipping RPC parity: MADARA_RPC_PARITY_JUNO_URL is not set");
            return Ok(());
        };
        let Some(pathfinder_url) = required_env("MADARA_RPC_PARITY_PATHFINDER_URL") else {
            eprintln!("skipping RPC parity: MADARA_RPC_PARITY_PATHFINDER_URL is not set");
            return Ok(());
        };

        let blocks = parse_csv_u64("MADARA_RPC_PARITY_BLOCKS", &[2, 19]);
        let txs = parse_csv_strings("MADARA_RPC_PARITY_TXS", &[DEFAULT_TX]);
        let max_block = *blocks.iter().max().expect("parity blocks should not be empty");
        let max_block_arg = max_block.to_string();

        let mut madara = MadaraCmdBuilder::new()
            .args(["--full", "--network", "sepolia", "--sync-stop-at", max_block_arg.as_str(), "--no-l1-sync"])
            .label("rpc-parity")
            .run();
        madara.wait_for_ready().await;
        madara.wait_for_sync_to(max_block).await;

        let madara_url = format!("{}{}", madara.rpc_url().trim_end_matches('/'), PARITY_ROUTE);
        let providers =
            [Provider { name: "juno", url: juno_url }, Provider { name: "pathfinder", url: pathfinder_url }];
        let allowlist = allowlist();
        let mut unallowlisted_reports = Vec::new();
        let mut all_reports = Vec::new();

        for case in parity_cases(&blocks, &txs) {
            let madara_response =
                normalize_response(rpc_response(&madara_url, &case.method, case.params.clone()).await?);
            for provider in &providers {
                let reference_response =
                    normalize_response(rpc_response(&provider.url, &case.method, case.params.clone()).await?);
                let mut diffs = Vec::new();
                diff_values("$", &madara_response, &reference_response, &mut diffs);

                if diffs.is_empty() {
                    continue;
                }

                let unallowlisted = diffs
                    .iter()
                    .filter(|diff| !is_allowed(&allowlist, provider.name, &case.method, &diff.path))
                    .cloned()
                    .collect::<Vec<_>>();
                let report = DiffReport {
                    provider: provider.name.to_string(),
                    version: PARITY_VERSION.to_string(),
                    method: case.method.clone(),
                    params: case.params.clone(),
                    diffs,
                };
                if !unallowlisted.is_empty() {
                    unallowlisted_reports.push(DiffReport {
                        provider: provider.name.to_string(),
                        version: PARITY_VERSION.to_string(),
                        method: case.method.clone(),
                        params: case.params.clone(),
                        diffs: unallowlisted,
                    });
                }
                all_reports.push(report);
            }
        }

        write_report(&report_dir(), &all_reports)?;
        assert!(
            unallowlisted_reports.is_empty(),
            "RPC parity found unallowlisted diffs: {}",
            serde_json::to_string_pretty(&unallowlisted_reports)?
        );

        Ok(())
    }
}
