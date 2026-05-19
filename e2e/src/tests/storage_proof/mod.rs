use reqwest::Url;
use serde_json::{json, Value};

#[derive(Debug, Clone)]
struct ContractKeys {
    contract_address: String,
    storage_keys: Vec<String>,
}

pub async fn assert_storage_proofs_match_for_changed_keys(
    madara_endpoint: Url,
    pathfinder_endpoint: Url,
) -> Result<(), String> {
    let latest_block = latest_block_number(pathfinder_endpoint.clone()).await?;

    let mut compared_contracts = 0usize;
    let mut compared_blocks = 0usize;

    for block_number in 0..=latest_block {
        let state_update =
            rpc_call(pathfinder_endpoint.clone(), "starknet_getStateUpdate", json!([{ "block_number": block_number }]))
                .await?;

        let changed_contracts = extract_changed_contracts(&state_update)?;
        if changed_contracts.is_empty() {
            continue;
        }
        compared_blocks += 1;

        for contract in changed_contracts {
            let contract_address = contract.contract_address.clone();
            let storage_keys = contract.storage_keys.clone();
            let request = json!({
                "block_id": { "block_number": block_number },
                "contract_addresses": [contract_address.clone()],
                "contracts_storage_keys": [{
                    "contract_address": contract_address.clone(),
                    "storage_keys": storage_keys.clone(),
                }]
            });

            let madara_proof = rpc_call(madara_endpoint.clone(), "starknet_getStorageProof", request.clone()).await?;
            let pathfinder_proof = rpc_call(pathfinder_endpoint.clone(), "starknet_getStorageProof", request).await?;

            assert_json_eq(
                &madara_proof["global_roots"],
                &pathfinder_proof["global_roots"],
                block_number,
                &contract_address,
                "global_roots",
            )?;

            let madara_leaf = single_contract_leaf(&madara_proof)?;
            let pathfinder_leaf = single_contract_leaf(&pathfinder_proof)?;
            assert_json_eq(&madara_leaf, &pathfinder_leaf, block_number, &contract_address, "contract_leaf")?;

            for key in &storage_keys {
                let storage_request = json!([contract_address.clone(), key, { "block_number": block_number }]);
                let madara_value =
                    rpc_call(madara_endpoint.clone(), "starknet_getStorageAt", storage_request.clone()).await?;
                let pathfinder_value =
                    rpc_call(pathfinder_endpoint.clone(), "starknet_getStorageAt", storage_request).await?;

                assert_json_eq(
                    &madara_value,
                    &pathfinder_value,
                    block_number,
                    &contract_address,
                    &format!("storage_value[{key}]"),
                )?;
            }

            compared_contracts += 1;
        }
    }

    if compared_contracts == 0 {
        return Err("No changed contracts were compared in the E2E storage proof helper".to_string());
    }

    println!("Compared storage proofs for {} contracts across {} blocks", compared_contracts, compared_blocks);

    Ok(())
}

async fn latest_block_number(endpoint: Url) -> Result<u64, String> {
    let result = rpc_call(endpoint, "starknet_blockNumber", json!([])).await?;
    result.as_u64().ok_or_else(|| format!("starknet_blockNumber returned a non-u64 result: {result}"))
}

async fn rpc_call(endpoint: Url, method: &str, params: Value) -> Result<Value, String> {
    let response = reqwest::Client::new()
        .post(endpoint)
        .header("accept", "application/json")
        .header("content-type", "application/json")
        .json(&json!({
            "id": 1,
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        }))
        .send()
        .await
        .map_err(|err| format!("RPC call to {method} failed: {err}"))?;

    let body: Value = response.json().await.map_err(|err| format!("Invalid JSON response for {method}: {err}"))?;
    if let Some(error) = body.get("error") {
        return Err(format!("RPC {method} returned error: {error}"));
    }

    body.get("result").cloned().ok_or_else(|| format!("RPC {method} response is missing a result field"))
}

fn extract_changed_contracts(state_update: &Value) -> Result<Vec<ContractKeys>, String> {
    let storage_diffs = state_update
        .get("state_diff")
        .and_then(|value| value.get("storage_diffs"))
        .and_then(Value::as_array)
        .ok_or_else(|| "state_update is missing state_diff.storage_diffs".to_string())?;

    let mut contracts = Vec::new();
    for diff in storage_diffs {
        let contract_address = diff
            .get("address")
            .and_then(Value::as_str)
            .ok_or_else(|| "storage diff is missing address".to_string())?
            .to_string();

        let storage_keys = diff
            .get("storage_entries")
            .and_then(Value::as_array)
            .ok_or_else(|| "storage diff is missing storage_entries".to_string())?
            .iter()
            .map(|entry| {
                entry
                    .get("key")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .ok_or_else(|| "storage entry is missing key".to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;

        if !storage_keys.is_empty() {
            contracts.push(ContractKeys { contract_address, storage_keys });
        }
    }

    Ok(contracts)
}

fn single_contract_leaf(proof: &Value) -> Result<Value, String> {
    let leaf = proof
        .get("contracts_proof")
        .and_then(|value| value.get("contract_leaves_data"))
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .ok_or_else(|| "storage proof is missing contract_leaves_data[0]".to_string())?;

    Ok(json!({
        "class_hash": leaf
            .get("class_hash")
            .cloned()
            .ok_or_else(|| "contract leaf is missing class_hash".to_string())?,
        "nonce": leaf
            .get("nonce")
            .cloned()
            .ok_or_else(|| "contract leaf is missing nonce".to_string())?,
        "storage_root": leaf
            .get("storage_root")
            .cloned()
            .ok_or_else(|| "contract leaf is missing storage_root".to_string())?,
    }))
}

fn assert_json_eq(
    left: &Value,
    right: &Value,
    block_number: u64,
    contract_address: &str,
    field: &str,
) -> Result<(), String> {
    if left == right {
        return Ok(());
    }

    Err(format!(
        "Mismatch for {field} at block {block_number} contract {contract_address}\nleft: {left}\nright: {right}"
    ))
}
