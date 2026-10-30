use crate::core::config::StarknetVersion;
use crate::error::job::JobError;
use crate::error::other::OtherError;
use crate::utils::rest_client::RestClient;
use crate::worker::event_handler::triggers::snos::fetch_block_starknet_version;
use blockifier::bouncer::{snos_full_output_cells_from_counts, BouncerWeights};
use color_eyre::eyre::eyre;
use serde::Deserialize;
use starknet::core::types::MaybePreConfirmedStateUpdate;
use starknet::core::types::{BlockId, StateUpdate};
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::{JsonRpcClient, Provider};
use std::collections::HashSet;
use std::sync::Arc;
use tracing::debug;

#[derive(Deserialize)]
struct SnosOutputLimit {
    max_snos_full_output_cells: usize,
}

pub async fn get_block_snos_output_budget(
    block_number: u64,
    provider: &Arc<JsonRpcClient<HttpTransport>>,
    fgw: &Arc<RestClient>,
    message_segment_length: usize,
) -> Result<(usize, usize), JobError> {
    let state_update = provider
        .get_state_update(BlockId::Number(block_number))
        .await
        .map_err(|error| JobError::ProviderError(error.to_string()))?;
    let MaybePreConfirmedStateUpdate::Update(state_update) = state_update else {
        return Err(JobError::Other(OtherError(eyre!(
            "Block {} is still preconfirmed while computing its SNOS output budget",
            block_number
        ))));
    };
    let response = fgw
        .get("/feeder_gateway/get_snos_output_limit")
        .await
        .map_err(|error| JobError::Other(OtherError(eyre!("Failed to fetch SNOS output limit: {}", error))))?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(JobError::Other(OtherError(eyre!(
            "SNOS output limit request failed with status {}: {}",
            status,
            body
        ))));
    }
    let limit: SnosOutputLimit = response
        .json()
        .await
        .map_err(|error| JobError::Other(OtherError(eyre!("Failed to parse SNOS output limit response: {}", error))))?;
    Ok((snos_output_cells(&state_update, message_segment_length)?, limit.max_snos_full_output_cells))
}

fn snos_output_cells(state_update: &StateUpdate, message_segment_length: usize) -> Result<usize, JobError> {
    let state_diff = &state_update.state_diff;
    let storage_updates = state_diff.storage_diffs.iter().map(|diff| diff.storage_entries.len()).sum::<usize>();
    let mut contracts = HashSet::new();
    contracts.extend(state_diff.storage_diffs.iter().map(|diff| diff.address));
    contracts.extend(state_diff.nonces.iter().map(|nonce| nonce.contract_address));
    contracts.extend(state_diff.deployed_contracts.iter().map(|contract| contract.address));
    contracts.extend(state_diff.replaced_classes.iter().map(|class| class.contract_address));
    let declared_classes = state_diff.declared_classes.len()
        + state_diff.deprecated_declared_classes.len()
        + state_diff.migrated_compiled_classes.as_ref().map_or(0, Vec::len);

    snos_full_output_cells_from_counts(message_segment_length, contracts.len(), storage_updates, declared_classes)
        .ok_or_else(|| JobError::Other(OtherError(eyre!("SNOS output cell count overflow"))))
}

pub async fn get_block_builtin_weights(
    block_number: u64,
    fgw: &Arc<RestClient>,
    empty_block_proving_gas: u64,
) -> Result<BouncerWeights, JobError> {
    debug!(
        block_number = %block_number,
        "Requesting block bouncer weights via REST"
    );

    // Use the RestClient with query parameters
    let response = fgw
        .get(&format!("/feeder_gateway/get_block_bouncer_weights?blockNumber={}", block_number))
        .await
        .map_err(|e| JobError::Other(OtherError(eyre!("Failed to send REST request: {}", e))))?;

    // Check for HTTP errors
    if !response.status().is_success() {
        let status = response.status();
        let error_text = response.text().await.unwrap_or_else(|_| "Unable to read error response".to_string());
        return Err(JobError::Other(OtherError(eyre!("REST request failed with status {}: {}", status, error_text))));
    }

    // Parse the response
    let mut bouncer_weights: BouncerWeights = response
        .json()
        .await
        .map_err(|e| JobError::Other(OtherError(eyre!("Failed to parse REST response: {}", e))))?;

    // If proving_gas is zero (empty block), use the configured default value.
    // Every block has some proving cost regardless of transactions.
    if bouncer_weights.proving_gas.0 == 0 {
        debug!(
            block_number = %block_number,
            default_proving_gas = %empty_block_proving_gas,
            "Block has zero proving_gas (empty block), using default value"
        );
        bouncer_weights.proving_gas = starknet_api::execution_resources::GasAmount(empty_block_proving_gas);
    }

    Ok(bouncer_weights)
}

pub async fn get_block_version(
    block_num: u64,
    provider: &Arc<JsonRpcClient<HttpTransport>>,
) -> Result<StarknetVersion, JobError> {
    fetch_block_starknet_version(provider, block_num).await.map_err(|e| {
        JobError::Other(OtherError(eyre!("Failed to fetch Starknet version for block {}: {}", block_num, e)))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use starknet_core::types::{
        ContractStorageDiffItem, DeclaredClassItem, DeployedContractItem, MigratedCompiledClassItem, NonceUpdate,
        ReplacedClassItem, StateDiff, StorageEntry,
    };
    use starknet_types_core::felt::Felt;

    fn empty_state_diff() -> StateDiff {
        StateDiff {
            storage_diffs: vec![],
            deprecated_declared_classes: vec![],
            declared_classes: vec![],
            deployed_contracts: vec![],
            replaced_classes: vec![],
            nonces: vec![],
            migrated_compiled_classes: None,
        }
    }

    #[test]
    fn empty_state_update_output_is_fixed_header_plus_messages() {
        let update = StateUpdate {
            block_hash: Felt::ZERO,
            old_root: Felt::ZERO,
            new_root: Felt::ZERO,
            state_diff: empty_state_diff(),
        };

        assert_eq!(snos_output_cells(&update, 11).unwrap(), 25);
    }

    #[test]
    fn output_count_matches_serialized_state_diff_terms() {
        let mut state_diff = empty_state_diff();
        state_diff.storage_diffs = vec![
            ContractStorageDiffItem {
                address: Felt::ONE,
                storage_entries: vec![
                    StorageEntry { key: Felt::from(10), value: Felt::from(11) },
                    StorageEntry { key: Felt::from(12), value: Felt::from(13) },
                ],
            },
            ContractStorageDiffItem {
                address: Felt::TWO,
                storage_entries: vec![StorageEntry { key: Felt::from(20), value: Felt::from(21) }],
            },
        ];
        state_diff.nonces = vec![NonceUpdate { contract_address: Felt::THREE, nonce: Felt::from(30) }];
        state_diff.deployed_contracts =
            vec![DeployedContractItem { address: Felt::from(4), class_hash: Felt::from(40) }];
        // Address 2 is already present in storage diffs and must only count once.
        state_diff.replaced_classes =
            vec![ReplacedClassItem { contract_address: Felt::TWO, class_hash: Felt::from(200) }];
        state_diff.declared_classes = vec![
            DeclaredClassItem { class_hash: Felt::from(50), compiled_class_hash: Felt::from(51) },
            DeclaredClassItem { class_hash: Felt::from(52), compiled_class_hash: Felt::from(53) },
        ];
        state_diff.deprecated_declared_classes = vec![Felt::from(60)];
        state_diff.migrated_compiled_classes =
            Some(vec![MigratedCompiledClassItem { class_hash: Felt::from(70), compiled_class_hash: Felt::from(71) }]);
        let update = StateUpdate { block_hash: Felt::ZERO, old_root: Felt::ZERO, new_root: Felt::ZERO, state_diff };

        // 14 fixed + 7 messages + 4 contracts*6 + 3 storage*3 + 4 classes*3.
        assert_eq!(snos_output_cells(&update, 7).unwrap(), 66);
    }
}
