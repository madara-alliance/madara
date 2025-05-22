use color_eyre::Result;
use futures::{stream, StreamExt};
use starknet::core::types::{
    ContractStorageDiffItem, DeployedContractItem, Felt, NonceUpdate, ReplacedClassItem, StateUpdate, StorageEntry,
};
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::{JsonRpcClient, Provider, ProviderError};
use starknet_core::types::{BlockId, StarknetError};
use std::collections::HashMap;
use std::sync::Arc;

const SPECIAL_ADDRESS: &str = "0x2";
const MAPPING_START: &str = "0x80";
const GLOBAL_COUNTER_SLOT: &str = "0x0";

/// Represents a mapping from one value to another
#[derive(Debug, Clone)]
struct ValueMapping {
    provider: Arc<JsonRpcClient<HttpTransport>>,
    mappings: HashMap<Felt, Felt>,
    pre_range_block: u64,
}

impl ValueMapping {
    /// Creates a new ValueMapping from storage entries at the special address
    fn from_special_address(
        state_update: &StateUpdate,
        pre_range_block: u64,
        provider: &Arc<JsonRpcClient<HttpTransport>>,
    ) -> Self {
        let mut mappings = HashMap::new();

        // Find the special address storage entries
        if let Some(special_contract) = state_update
            .state_diff
            .storage_diffs
            .iter()
            .find(|diff| diff.address == Felt::from_hex(SPECIAL_ADDRESS).unwrap())
        {
            // Add each key-value pair to our mapping, ignoring the global counter-slot
            for entry in &special_contract.storage_entries {
                if entry.key != Felt::from_hex(GLOBAL_COUNTER_SLOT).unwrap() {
                    mappings.insert(entry.key, entry.value);
                }
            }
        }

        ValueMapping { mappings, provider: provider.clone(), pre_range_block }
    }

    async fn get_value(&self, value: &Felt) -> Result<Felt, ProviderError> {
        match self
            .provider
            .get_storage_at(Felt::from_hex(SPECIAL_ADDRESS).unwrap(), value, BlockId::Number(self.pre_range_block))
            .await
        {
            Ok(value) => {
                Ok(value)
            }
            Err(e) => Err(ProviderError::StarknetError(StarknetError::UnexpectedError(format!(
                "Failed to get pre-range storage value for contract: {}, key: {} at block {}: {}",
                SPECIAL_ADDRESS, value, self.pre_range_block, e
            )))),
        }
    }

    /// Maps a value using the stored mappings
    // async fn map_value(&self, value: &Felt) -> Result<Felt> {
    //     Ok(self.mappings.get(value).cloned().unwrap_or(*value))
    // }

    async fn map_value(&mut self, value: &Felt) -> Result<Felt, ProviderError> {
        if *value < Felt::from_hex(MAPPING_START).unwrap() {
            return Ok(value.clone());
        }
        match self.mappings.get(value).cloned() {
            Some(value) => Ok(value),
            None => {
                let key = self.get_value(value).await?;
                self.mappings.insert(key, value.clone());
                Ok(key)
            }
        }
    }
}

/// Compresses a state update using stateful compression
///
/// This function:
/// 1. Extracts mapping information from the special address (0x2)
/// 2. Use this mapping to transform other addresses and values
/// 3. Preserves the special address entries as-is
///
/// # Arguments
/// * `state_update` - StateUpdate to compress
///
/// # Returns
/// A compressed StateUpdate with values mapped according to the special address mappings
pub async fn compress(
    state_update: &StateUpdate,
    pre_range_block: u64,
    provider: &Arc<JsonRpcClient<HttpTransport>>,
) -> Result<StateUpdate> {
    let mut state_update = state_update.clone();

    // Create mapping from the special address
    let mut mapping = ValueMapping::from_special_address(&state_update, pre_range_block, provider);

    // Create a new storage diffs vector
    let mut new_storage_diffs = Vec::new();

    // Process each contract's storage diffs
    for diff in state_update.state_diff.storage_diffs {
        if skip_contract(diff.address) {
            // Preserve special address entries as-is
            new_storage_diffs.push(diff);
            continue;
        }

        // Map the contract address
        // println!("getting mapping for contract {}", diff.address);
        let mapped_address = mapping.map_value(&diff.address).await?;

        // Map storage entries
        let mapped_entry_results: Vec<_> = stream::iter(diff.storage_entries)
            .map(|entry| {
                let mut mapping = mapping.clone();
                async move { Ok(StorageEntry { key: mapping.map_value(&entry.key).await?, value: entry.value }) }
            })
            .buffer_unordered(4000)
            .collect()
            .await;

        // Create a new storage diff with mapped values
        new_storage_diffs.push(ContractStorageDiffItem {
            address: mapped_address,
            storage_entries: parse_results(mapped_entry_results)?,
        });
    }

    // Update storage diffs in state update
    state_update.state_diff.storage_diffs = new_storage_diffs;

    // Map other fields that need mapping
    let deployed_contract_results: Vec<_> = stream::iter(state_update.state_diff.deployed_contracts)
        .map(|item| {
            let mut mapping = mapping.clone();
            async move {
                Ok(DeployedContractItem {
                    address: mapping.map_value(&item.address).await?,
                    // class_hash: mapping.map_value(&item.class_hash).await?,
                    class_hash: item.class_hash,
                })
            }
        })
        .buffer_unordered(4000)
        .collect()
        .await;

    // let declared_class_results: Vec<_> = stream::iter(state_update.state_diff.declared_classes)
    //     .map(|item| {
    //         let mut mapping = mapping.clone();
    //         async move {
    //             Ok(DeclaredClassItem {
    //                 class_hash: mapping.map_value(&item.class_hash).await?,
    //                 compiled_class_hash: mapping.map_value(&item.compiled_class_hash).await?,
    //             })
    //         }
    //     })
    //     .buffer_unordered(100)
    //     .collect()
    //     .await;

    // Map nonces
    let nonce_results: Vec<_> = stream::iter(state_update.state_diff.nonces)
        .map(|item| {
            let mut mapping = mapping.clone();
            async move {
                Ok(NonceUpdate {
                    contract_address: mapping.map_value(&item.contract_address).await?,
                    // nonce: mapping.map_value(&item.nonce).await?,
                    nonce: item.nonce,
                })
            }
        })
        .buffer_unordered(4000)
        .collect()
        .await;

    // Map replaced classes
    let replaced_class_results: Vec<_> = stream::iter(state_update.state_diff.replaced_classes)
        .map(|item| {
            let mut mapping = mapping.clone();
            async move {
                Ok(ReplacedClassItem {
                    contract_address: mapping.map_value(&item.contract_address).await?,
                    // class_hash: mapping.map_value(&item.class_hash).await?,
                    class_hash: item.class_hash,
                })
            }
        })
        .buffer_unordered(4000)
        .collect()
        .await;

    // Map deprecated declared classes
    // let deprecated_declared_class_results: Vec<_> = stream::iter(state_update.state_diff.deprecated_declared_classes)
    //     .map(|class_hash| {
    //         let mut mapping = mapping.clone();
    //         async move { Ok(mapping.map_value(&class_hash).await?) }
    //     })
    //     .buffer_unordered(100)
    //     .collect()
    //     .await;

    state_update.state_diff.deployed_contracts = parse_results(deployed_contract_results)?;
    // state_update.state_diff.declared_classes = parse_results(declared_class_results)?;
    state_update.state_diff.nonces = parse_results(nonce_results)?;
    state_update.state_diff.replaced_classes = parse_results(replaced_class_results)?;
    // state_update.state_diff.deprecated_declared_classes = parse_results(deprecated_declared_class_results)?;

    // Map block hashes and roots if needed
    state_update.block_hash = state_update.block_hash;
    state_update.new_root = state_update.new_root;
    state_update.old_root = state_update.old_root;

    Ok(state_update)
}

fn skip_contract(address: Felt) -> bool {
    let res = address < Felt::from_hex(MAPPING_START).unwrap();
    // println!("skipping contract {}? {}", address, res);
    res
}

fn parse_results<T>(results: Vec<Result<T>>) -> Result<Vec<T>> {
    let mut values = Vec::new();
    for result in results {
        match result {
            Ok(value) => {
                values.push(value);
            }
            Err(e) => return Err(e),
        }
    }
    Ok(values)
}
