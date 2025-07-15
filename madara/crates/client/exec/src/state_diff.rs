use blockifier::state::cached_state::StateMaps;
use itertools::{Either, Itertools};
use mc_db::{db_block_id::DbBlockId, MadaraBackend};
use mp_convert::{Felt, ToFelt};
use mp_state_update::{
    ContractStorageDiffItem, DeclaredClassItem, DeployedContractItem, NonceUpdate, ReplacedClassItem, StateDiff,
    StorageEntry,
};
use std::collections::HashMap;

/// Create a state diff out of a merged state map.
/// The resulting state diff is normalized: when merging state maps together, if for example a transaction changed a
/// given storage key to new value, and a following transaction change it back to the original value - the normalized state diff
/// should not preserve the storage key, since it ends up not being changed in the end. This function does this normalization,
/// by looking up every key in the backend and only keep the changed ones.
pub fn create_normalized_state_diff(
    backend: &MadaraBackend,
    on_top_of: &Option<DbBlockId>,
    unnormalized_state_map: StateMaps,
) -> anyhow::Result<StateDiff> {
    fn sorted_by_key<T, K: Ord, F: FnMut(&T) -> K>(mut vec: Vec<T>, f: F) -> Vec<T> {
        vec.sort_by_key(f);
        vec
    }

    // Filter and group storage diffs.
    // HashMap<(Addr, K), V> => HashMap<Addr, Vec<(K, V)>>
    let storage_diffs = unnormalized_state_map.storage.into_iter().try_fold(
        HashMap::<Felt, Vec<StorageEntry>>::new(),
        |mut acc, ((addr, key), value)| -> anyhow::Result<_> {
            let previous_value =
                backend.get_contract_storage_at(on_top_of, &addr.to_felt(), &key.to_felt())?.unwrap_or_default();
            if previous_value != value {
                // only keep changed keys.
                acc.entry(addr.to_felt()).or_default().push(StorageEntry { key: key.to_felt(), value });
            }
            Ok(acc)
        },
    )?;
    // Map into a sorted vec.
    let storage_diffs: Vec<ContractStorageDiffItem> = sorted_by_key(
        storage_diffs
            .into_iter()
            .map(|(address, entries)| ContractStorageDiffItem {
                address,
                storage_entries: sorted_by_key(entries, |entry| entry.key),
            })
            .collect(),
        |entry| entry.address,
    );

    // We shouldn't need to check the backend for duplicate here: you can't redeclare a class (except on very early mainnet blocks, not handled here).
    let (declared_classes, deprecated_declared_classes): (Vec<DeclaredClassItem>, Vec<Felt>) =
        unnormalized_state_map.declared_contracts.into_iter().partition_map(|(class_hash, _)| {
            if let Some(compiled_hash) = unnormalized_state_map.compiled_class_hashes.get(&class_hash) {
                // Sierra
                Either::Left(DeclaredClassItem {
                    class_hash: class_hash.to_felt(),
                    compiled_class_hash: compiled_hash.to_felt(),
                })
            } else {
                // Legacy
                Either::Right(class_hash.to_felt())
            }
        });
    let (declared_classes, deprecated_declared_classes) = (
        sorted_by_key(declared_classes, |entry| entry.class_hash),
        sorted_by_key(deprecated_declared_classes, |class_hash| *class_hash),
    );

    // Same here: duplicate is not possible for nonces.
    let nonces = sorted_by_key(
        unnormalized_state_map
            .nonces
            .into_iter()
            .map(|(contract_address, nonce)| NonceUpdate {
                contract_address: contract_address.to_felt(),
                nonce: nonce.to_felt(),
            })
            .collect(),
        |entry| entry.contract_address,
    );

    // Differentiate between: newly deployed contract, replaced class entry. Remove entries where no change happened.
    let (deployed_contracts, replaced_classes): (Vec<DeployedContractItem>, Vec<ReplacedClassItem>) =
        unnormalized_state_map.class_hashes.into_iter().try_fold(
            (Vec::new(), Vec::new()),
            |(mut deployed, mut replaced), (contract_address, class_hash)| -> anyhow::Result<_> {
                let previous_class_hash =
                    backend.get_contract_class_hash_at(on_top_of, &contract_address.to_felt())?.unwrap_or(Felt::ZERO);
                if previous_class_hash == Felt::ZERO {
                    // Newly deployed contract
                    deployed.push(DeployedContractItem {
                        address: contract_address.to_felt(),
                        class_hash: class_hash.to_felt(),
                    });
                } else if previous_class_hash != class_hash.to_felt() {
                    // Replaced class
                    replaced.push(ReplacedClassItem {
                        contract_address: contract_address.to_felt(),
                        class_hash: class_hash.to_felt(),
                    });
                }

                Ok((deployed, replaced))
            },
        )?;
    let (deployed_contracts, replaced_classes) = (
        sorted_by_key(deployed_contracts, |entry| entry.address),
        sorted_by_key(replaced_classes, |entry| entry.contract_address),
    );

    Ok(StateDiff {
        storage_diffs,
        deprecated_declared_classes,
        declared_classes,
        nonces,
        deployed_contracts,
        replaced_classes,
    })
}

#[cfg(test)]
mod tests {
    use blockifier::state::cached_state::StateMaps;
    use itertools::Itertools;
    use mc_db::{db_block_id::DbBlockId, MadaraBackend};
    use mp_block::{header::PendingHeader, PendingFullBlock};
    use mp_chain_config::ChainConfig;
    use mp_convert::{Felt, ToFelt};
    use mp_state_update::{
        ContractStorageDiffItem, DeclaredClassItem, DeployedContractItem, NonceUpdate, ReplacedClassItem, StateDiff,
        StorageEntry,
    };
    use starknet_api::{
        core::{ClassHash, CompiledClassHash, ContractAddress, Nonce},
        state::StorageKey,
    };
    use std::{collections::HashMap, sync::Arc};

    #[tokio::test]
    async fn test_create_normalized_state_diff() {
        let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));

        // Set up initial backend state
        let addr1 = ContractAddress::try_from(Felt::from(1)).unwrap();
        let addr2 = ContractAddress::try_from(Felt::from(0x2)).unwrap();
        let addr3 = ContractAddress::try_from(Felt::from(0x3)).unwrap();
        let key1 = StorageKey::try_from(Felt::from(0x10)).unwrap();
        let key2 = StorageKey::try_from(Felt::from(0x20)).unwrap();
        let old_value1 = Felt::from(0x100);
        let old_value2 = Felt::from(0x200);
        let old_class_hash1 = ClassHash(Felt::from(0x1000));
        let old_class_hash2 = ClassHash(Felt::from(0x2000));

        backend
            .add_full_block_with_classes(
                PendingFullBlock {
                    header: PendingHeader::default(),
                    state_diff: StateDiff {
                        storage_diffs: vec![
                            ContractStorageDiffItem {
                                address: addr1.to_felt(),
                                storage_entries: vec![
                                    StorageEntry { key: key1.to_felt(), value: old_value1 },
                                    StorageEntry { key: key2.to_felt(), value: old_value2 },
                                ],
                            },
                            ContractStorageDiffItem {
                                address: addr2.to_felt(),
                                storage_entries: vec![StorageEntry { key: key1.to_felt(), value: old_value1 }],
                            },
                        ],
                        deprecated_declared_classes: vec![],
                        declared_classes: vec![],
                        deployed_contracts: vec![
                            DeployedContractItem { address: addr1.to_felt(), class_hash: old_class_hash1.to_felt() },
                            DeployedContractItem { address: addr2.to_felt(), class_hash: old_class_hash2.to_felt() },
                        ],
                        replaced_classes: vec![],
                        nonces: vec![],
                    },
                    transactions: vec![],
                    events: vec![],
                },
                0,
                &[],
                true,
            )
            .await
            .unwrap();

        // Create new state map with mixed changes
        let new_value1 = Felt::from(0x300);
        let new_value2 = Felt::from(0x400);
        let new_class_hash1 = ClassHash(Felt::from(0x3000));
        let new_class_hash2 = ClassHash(Felt::from(0x4000));
        let sierra_class_hash = ClassHash(Felt::from(0x5000));
        let sierra_compiled_hash = CompiledClassHash(Felt::from(0x5001));
        let legacy_class_hash = ClassHash(Felt::from(0x6000));

        let storage = HashMap::from([
            ((addr1, key1), old_value1), // unchanged - should be filtered
            ((addr1, key2), new_value1), // changed - should be kept
            ((addr2, key1), new_value2), // changed - should be kept
            ((addr3, key1), new_value1), // new contract - should be kept
        ]);

        let declared_contracts = HashMap::from([(sierra_class_hash, true), (legacy_class_hash, true)]);

        let compiled_class_hashes = HashMap::from([(sierra_class_hash, sierra_compiled_hash)]);

        let nonces = HashMap::from([(addr1, Nonce(Felt::from(0x5))), (addr3, Nonce(Felt::from(0x10)))]);

        let class_hashes = HashMap::from([
            (addr1, new_class_hash1), // replacement - different from old
            (addr2, old_class_hash2), // unchanged - should be filtered
            (addr3, new_class_hash2), // deployment - new contract
        ]);

        let unnormalized_state_map =
            StateMaps { storage, declared_contracts, compiled_class_hashes, nonces, class_hashes };
        let result =
            super::create_normalized_state_diff(&backend, &Some(DbBlockId::Number(0)), unnormalized_state_map).unwrap();

        // Check storage diffs - only changed entries should remain
        assert_eq!(
            result.storage_diffs,
            [
                ContractStorageDiffItem {
                    address: addr1.to_felt(),
                    storage_entries: vec![StorageEntry { key: key2.to_felt(), value: new_value1 }],
                },
                ContractStorageDiffItem {
                    address: addr2.to_felt(),
                    storage_entries: vec![StorageEntry { key: key1.to_felt(), value: new_value2 }],
                },
                ContractStorageDiffItem {
                    address: addr3.to_felt(),
                    storage_entries: vec![StorageEntry { key: key1.to_felt(), value: new_value1 }],
                },
            ]
            .into_iter()
            .sorted_by_key(|d| d.address)
            .collect::<Vec<_>>()
        );

        // Check declared classes
        assert_eq!(
            result.declared_classes,
            [DeclaredClassItem {
                class_hash: sierra_class_hash.to_felt(),
                compiled_class_hash: sierra_compiled_hash.to_felt()
            }]
            .into_iter()
            .sorted_by_key(|d| d.class_hash)
            .collect::<Vec<_>>()
        );

        assert_eq!(
            result.deprecated_declared_classes,
            [legacy_class_hash.to_felt()].into_iter().sorted().collect::<Vec<_>>()
        );

        // Check nonces
        assert_eq!(
            result.nonces,
            [
                NonceUpdate { contract_address: addr1.to_felt(), nonce: Felt::from(0x5) },
                NonceUpdate { contract_address: addr3.to_felt(), nonce: Felt::from(0x10) },
            ]
            .into_iter()
            .sorted_by_key(|n| n.contract_address)
            .collect::<Vec<_>>()
        );

        // Check deployments (only addr3 should be new deployment)
        assert_eq!(
            result.deployed_contracts,
            [DeployedContractItem { address: addr3.to_felt(), class_hash: new_class_hash2.to_felt() }]
                .into_iter()
                .sorted_by_key(|d| d.address)
                .collect::<Vec<_>>()
        );

        // Check replacements (only addr1 should have class replacement)
        assert_eq!(
            result.replaced_classes,
            [ReplacedClassItem { contract_address: addr1.to_felt(), class_hash: new_class_hash1.to_felt() }]
                .into_iter()
                .sorted_by_key(|r| r.contract_address)
                .collect::<Vec<_>>()
        );
    }
}
