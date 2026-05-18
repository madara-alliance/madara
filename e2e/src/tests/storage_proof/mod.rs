use crate::services::helpers::NodeRpcMethods;
use crate::setup::{ChainSetup, SetupConfigBuilder};
use crate::tests::deposit_withdraw::utils::cleanup_test_directory;
use reqwest::Url;
use rstest::*;
use serde_json::{json, Value};
use std::env;

#[derive(Debug, Clone)]
struct ContractKeys {
    contract_address: String,
    storage_keys: Vec<String>,
}

#[fixture]
pub async fn setup_chain_with_storage_proof(#[default("")] test_name: &str) -> ChainSetup {
    dotenvy::from_filename(".env.e2e").expect("Failed to load the .env file");

    env::set_var("MADARA_RPC_STORAGE_PROOF_MAX_DISTANCE", "10000");
    env::set_var("MADARA_ORCHESTRATOR_STORE_AUDIT_ARTIFACTS", "false");

    let setup_config = SetupConfigBuilder::new(None).test_config_l2(test_name).await.unwrap();
    let mut setup = ChainSetup::new(setup_config).unwrap();
    match setup.setup(test_name).await {
        Ok(()) => println!("✅ Storage proof setup completed successfully"),
        Err(e) => {
            println!("❌ Storage proof setup failed: {}", e);
            panic!("Setup failed: {}", e);
        }
    }

    setup
}

#[rstest]
#[case("storage_proof")]
#[tokio::test]
async fn test_storage_proof_matches_pathfinder_for_changed_keys(
    #[case] test_name: &str,
    #[future]
    #[with(test_name)]
    setup_chain_with_storage_proof: ChainSetup,
) {
    let result = run_storage_proof_comparison(setup_chain_with_storage_proof.await).await;

    cleanup_test_directory(test_name);

    match result {
        Ok(_) => println!("✅ Storage proof comparison completed successfully"),
        Err(e) => {
            eprintln!("❌ Storage proof comparison failed: {}", e);
            panic!("Test failed: {}", e);
        }
    }
}

async fn run_storage_proof_comparison(setup: ChainSetup) -> Result<(), String> {
    let services =
        setup.lifecycle_manager.services.as_ref().ok_or_else(|| "Runtime services are not available".to_string())?;

    let madara = services.madara_service.as_ref().ok_or_else(|| "Madara service is not available".to_string())?;
    let pathfinder =
        services.pathfinder_service.as_ref().ok_or_else(|| "Pathfinder service is not available".to_string())?;

    let latest_block = pathfinder
        .get_latest_block_number()
        .await
        .map_err(|err| format!("Failed to get latest block from Pathfinder: {err}"))?
        .ok_or_else(|| "No blocks available on Pathfinder".to_string())?;

    let mut compared_contracts = 0usize;
    let mut compared_blocks = 0usize;

    for block_number in 0..=latest_block {
        let state_update =
            rpc_call(pathfinder.endpoint(), "starknet_getStateUpdate", json!([{ "block_number": block_number }]))
                .await?;

        let changed_contracts = extract_changed_contracts(&state_update)?;
        if changed_contracts.is_empty() {
            continue;
        }
        compared_blocks += 1;

        for contract in changed_contracts {
            let contract_address = contract.contract_address.clone();
            let storage_keys = contract.storage_keys.clone();
            let madara_proof = rpc_call(
                madara.rpc_endpoint(),
                "starknet_getStorageProof",
                json!({
                    "block_id": { "block_number": block_number },
                    "contract_addresses": [contract_address.clone()],
                    "contracts_storage_keys": [{
                        "contract_address": contract_address.clone(),
                        "storage_keys": storage_keys.clone(),
                    }]
                }),
            )
            .await?;

            let pathfinder_proof = rpc_call(
                pathfinder.endpoint(),
                "starknet_getStorageProof",
                json!({
                    "block_id": { "block_number": block_number },
                    "contract_addresses": [contract_address.clone()],
                    "contracts_storage_keys": [{
                        "contract_address": contract_address.clone(),
                        "storage_keys": storage_keys.clone(),
                    }]
                }),
            )
            .await?;

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
                let madara_value = rpc_call(
                    madara.rpc_endpoint(),
                    "starknet_getStorageAt",
                    json!([contract_address.clone(), key, { "block_number": block_number }]),
                )
                .await?;
                let pathfinder_value = rpc_call(
                    pathfinder.endpoint(),
                    "starknet_getStorageAt",
                    json!([contract_address.clone(), key, { "block_number": block_number }]),
                )
                .await?;

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
        return Err("No changed contracts were compared in the E2E storage proof test".to_string());
    }

    println!("Compared storage proofs for {} contracts across {} blocks", compared_contracts, compared_blocks);

    Ok(())
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

#[allow(dead_code)]
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
