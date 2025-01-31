use crate::Error;
use blockifier::{
    blockifier::transaction_executor::{TransactionExecutor, BLOCK_STATE_ACCESS_ERR},
    bouncer::BouncerWeights,
    state::{cached_state::StateMaps, state_api::StateReader},
    transaction::errors::TransactionExecutionError,
};
use mc_db::{db_block_id::DbBlockId, MadaraBackend};
use mp_block::{VisitedSegmentEntry, VisitedSegments};
use mp_convert::ToFelt;
use mp_state_update::{
    ContractStorageDiffItem, DeclaredClassItem, DeployedContractItem, NonceUpdate, ReplacedClassItem, StateDiff,
    StorageEntry,
};
use starknet_api::core::ContractAddress;
use std::collections::{hash_map, HashMap};

#[derive(Debug, thiserror::Error)]
#[error("Error converting state diff to state map")]
pub struct StateDiffToStateMapError;

pub(crate) fn state_map_to_state_diff(
    backend: &MadaraBackend,
    on_top_of: &Option<DbBlockId>,
    diff: StateMaps,
) -> Result<StateDiff, Error> {
    let mut backing_map = HashMap::<ContractAddress, usize>::default();
    let mut storage_diffs = Vec::<ContractStorageDiffItem>::default();
    for ((address, key), value) in diff.storage {
        match backing_map.entry(address) {
            hash_map::Entry::Vacant(e) => {
                e.insert(storage_diffs.len());
                storage_diffs.push(ContractStorageDiffItem {
                    address: address.to_felt(),
                    storage_entries: vec![StorageEntry { key: key.to_felt(), value }],
                });
            }
            hash_map::Entry::Occupied(e) => {
                storage_diffs[*e.get()].storage_entries.push(StorageEntry { key: key.to_felt(), value });
            }
        }
    }

    let mut deprecated_declared_classes = Vec::default();
    for (class_hash, _) in diff.declared_contracts {
        if !diff.compiled_class_hashes.contains_key(&class_hash) {
            deprecated_declared_classes.push(class_hash.to_felt());
        }
    }

    let declared_classes = diff
        .compiled_class_hashes
        .iter()
        .map(|(class_hash, compiled_class_hash)| DeclaredClassItem {
            class_hash: class_hash.to_felt(),
            compiled_class_hash: compiled_class_hash.to_felt(),
        })
        .collect();

    let nonces = diff
        .nonces
        .into_iter()
        .map(|(contract_address, nonce)| NonceUpdate {
            contract_address: contract_address.to_felt(),
            nonce: nonce.to_felt(),
        })
        .collect();

    let mut deployed_contracts = Vec::new();
    let mut replaced_classes = Vec::new();
    for (contract_address, new_class_hash) in diff.class_hashes {
        let replaced = if let Some(on_top_of) = on_top_of {
            match backend.get_contract_class_hash_at(on_top_of, &contract_address.to_felt())? {
                Some(class_hash) => class_hash != new_class_hash.to_felt(),
                None => false,
            }
        } else {
            // Executing genesis block: nothing being redefined here
            false
        };
        if replaced {
            replaced_classes.push(ReplacedClassItem {
                contract_address: contract_address.to_felt(),
                class_hash: new_class_hash.to_felt(),
            })
        } else {
            deployed_contracts.push(DeployedContractItem {
                address: contract_address.to_felt(),
                class_hash: new_class_hash.to_felt(),
            })
        }
    }

    Ok(StateDiff {
        storage_diffs,
        deprecated_declared_classes,
        declared_classes,
        nonces,
        deployed_contracts,
        replaced_classes,
    })
}

fn get_visited_segments<S: StateReader>(tx_executor: &mut TransactionExecutor<S>) -> Result<VisitedSegments, Error> {
    let visited_segments = tx_executor
        .block_state
        .as_ref()
        .expect(BLOCK_STATE_ACCESS_ERR)
        .visited_pcs
        .iter()
        .map(|(class_hash, class_visited_pcs)| -> Result<_, Error> {
            let contract_class = tx_executor
                .block_state
                .as_ref()
                .expect(BLOCK_STATE_ACCESS_ERR)
                .get_compiled_contract_class(*class_hash)
                .map_err(TransactionExecutionError::StateError)?;
            Ok(VisitedSegmentEntry {
                class_hash: class_hash.to_felt(),
                segments: contract_class.get_visited_segments(class_visited_pcs)?,
            })
        })
        .collect::<Result<_, Error>>()?;

    Ok(VisitedSegments(visited_segments))
}

pub(crate) fn finalize_execution_state<S: StateReader>(
    tx_executor: &mut TransactionExecutor<S>,
    backend: &MadaraBackend,
    on_top_of: &Option<DbBlockId>,
) -> Result<(StateDiff, VisitedSegments, BouncerWeights), Error> {
    let state_map = tx_executor
        .block_state
        .as_mut()
        .expect(BLOCK_STATE_ACCESS_ERR)
        .to_state_diff()
        .map_err(TransactionExecutionError::StateError)?;
    let state_update = state_map_to_state_diff(backend, on_top_of, state_map)?;

    let visited_segments = get_visited_segments(tx_executor)?;

    Ok((state_update, visited_segments, *tx_executor.bouncer.get_accumulated_weights()))
}

#[cfg(test)]
mod test {
    use std::{collections::HashMap, sync::Arc};

    use blockifier::{compiled_class_hash, nonce, state::cached_state::StateMaps, storage_key};
    use mc_db::MadaraBackend;
    use mp_chain_config::ChainConfig;
    use mp_convert::ToFelt;
    use mp_state_update::{
        ContractStorageDiffItem, DeclaredClassItem, DeployedContractItem, NonceUpdate, StateDiff, StorageEntry,
    };
    use starknet_api::{
        class_hash, contract_address,
        core::{ClassHash, ContractAddress, PatriciaKey},
        felt, patricia_key,
    };
    use starknet_core::types::Felt;

    #[test]
    fn state_map_to_state_diff() {
        let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));

        let mut nonces = HashMap::new();
        nonces.insert(contract_address!(1u32), nonce!(1));
        nonces.insert(contract_address!(2u32), nonce!(2));
        nonces.insert(contract_address!(3u32), nonce!(3));

        let mut class_hashes = HashMap::new();
        class_hashes.insert(contract_address!(1u32), class_hash!("0xc1a551"));
        class_hashes.insert(contract_address!(2u32), class_hash!("0xc1a552"));
        class_hashes.insert(contract_address!(3u32), class_hash!("0xc1a553"));

        let mut storage = HashMap::new();
        storage.insert((contract_address!(1u32), storage_key!(1u32)), felt!(1u32));
        storage.insert((contract_address!(1u32), storage_key!(2u32)), felt!(2u32));
        storage.insert((contract_address!(1u32), storage_key!(3u32)), felt!(3u32));

        storage.insert((contract_address!(2u32), storage_key!(1u32)), felt!(1u32));
        storage.insert((contract_address!(2u32), storage_key!(2u32)), felt!(2u32));
        storage.insert((contract_address!(2u32), storage_key!(3u32)), felt!(3u32));

        storage.insert((contract_address!(3u32), storage_key!(1u32)), felt!(1u32));
        storage.insert((contract_address!(3u32), storage_key!(2u32)), felt!(2u32));
        storage.insert((contract_address!(3u32), storage_key!(3u32)), felt!(3u32));

        let mut compiled_class_hashes = HashMap::new();
        // "0xc1a553" is marked as deprecated by not having a compiled
        // class hashe
        compiled_class_hashes.insert(class_hash!("0xc1a551"), compiled_class_hash!(0x1));
        compiled_class_hashes.insert(class_hash!("0xc1a552"), compiled_class_hash!(0x2));

        let mut declared_contracts = HashMap::new();
        declared_contracts.insert(class_hash!("0xc1a551"), true);
        declared_contracts.insert(class_hash!("0xc1a552"), true);
        declared_contracts.insert(class_hash!("0xc1a553"), true);

        let state_map = StateMaps { nonces, class_hashes, storage, compiled_class_hashes, declared_contracts };

        let storage_diffs = vec![
            ContractStorageDiffItem {
                address: felt!(1u32),
                storage_entries: vec![
                    StorageEntry { key: felt!(1u32), value: Felt::ONE },
                    StorageEntry { key: felt!(2u32), value: Felt::TWO },
                    StorageEntry { key: felt!(3u32), value: Felt::THREE },
                ],
            },
            ContractStorageDiffItem {
                address: felt!(2u32),
                storage_entries: vec![
                    StorageEntry { key: felt!(1u32), value: Felt::ONE },
                    StorageEntry { key: felt!(2u32), value: Felt::TWO },
                    StorageEntry { key: felt!(3u32), value: Felt::THREE },
                ],
            },
            ContractStorageDiffItem {
                address: felt!(3u32),
                storage_entries: vec![
                    StorageEntry { key: felt!(1u32), value: Felt::ONE },
                    StorageEntry { key: felt!(2u32), value: Felt::TWO },
                    StorageEntry { key: felt!(3u32), value: Felt::THREE },
                ],
            },
        ];

        let deprecated_declared_classes = vec![class_hash!("0xc1a553").to_felt()];

        let declared_classes = vec![
            DeclaredClassItem {
                class_hash: class_hash!("0xc1a551").to_felt(),
                compiled_class_hash: compiled_class_hash!(0x1).to_felt(),
            },
            DeclaredClassItem {
                class_hash: class_hash!("0xc1a552").to_felt(),
                compiled_class_hash: compiled_class_hash!(0x2).to_felt(),
            },
        ];

        let nonces = vec![
            NonceUpdate { contract_address: felt!(1u32), nonce: felt!(1u32) },
            NonceUpdate { contract_address: felt!(2u32), nonce: felt!(2u32) },
            NonceUpdate { contract_address: felt!(3u32), nonce: felt!(3u32) },
        ];

        let deployed_contracts = vec![
            DeployedContractItem { address: felt!(1u32), class_hash: class_hash!("0xc1a551").to_felt() },
            DeployedContractItem { address: felt!(2u32), class_hash: class_hash!("0xc1a552").to_felt() },
            DeployedContractItem { address: felt!(3u32), class_hash: class_hash!("0xc1a553").to_felt() },
        ];

        let replaced_classes = vec![];

        let expected = StateDiff {
            storage_diffs,
            deprecated_declared_classes,
            declared_classes,
            nonces,
            deployed_contracts,
            replaced_classes,
        };

        let mut actual = super::state_map_to_state_diff(&backend, &Option::<_>::None, state_map).unwrap();

        actual.storage_diffs.sort_by(|a, b| a.address.cmp(&b.address));
        actual.storage_diffs.iter_mut().for_each(|s| s.storage_entries.sort_by(|a, b| a.key.cmp(&b.key)));
        actual.deprecated_declared_classes.sort();
        actual.declared_classes.sort_by(|a, b| a.class_hash.cmp(&b.class_hash));
        actual.nonces.sort_by(|a, b| a.contract_address.cmp(&b.contract_address));
        actual.deployed_contracts.sort_by(|a, b| a.address.cmp(&b.address));
        actual.replaced_classes.sort_by(|a, b| a.contract_address.cmp(&b.contract_address));

        assert_eq!(
            actual,
            expected,
            "actual: {}\nexpected: {}",
            serde_json::to_string_pretty(&actual).unwrap_or_default(),
            serde_json::to_string_pretty(&expected).unwrap_or_default()
        );
    }
}
