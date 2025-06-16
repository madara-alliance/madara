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
mod test {
    use blockifier::state::cached_state::StateMaps;
    use mc_db::MadaraBackend;
    use mp_chain_config::ChainConfig;
    use mp_convert::Felt;
    use mp_state_update::{
        ContractStorageDiffItem, DeclaredClassItem, DeployedContractItem, NonceUpdate, StateDiff, StorageEntry,
    };
    use starknet_api::core::{ClassHash, CompiledClassHash, Nonce};
    use std::{collections::HashMap, sync::Arc};

    #[test]
    fn state_map_to_state_diff() {
        let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));

        let mut nonces = HashMap::new();
        nonces.insert(Felt::from_hex_unchecked("1").try_into().unwrap(), Nonce(Felt::from_hex_unchecked("1")));
        nonces.insert(Felt::from_hex_unchecked("2").try_into().unwrap(), Nonce(Felt::from_hex_unchecked("2")));
        nonces.insert(Felt::from_hex_unchecked("3").try_into().unwrap(), Nonce(Felt::from_hex_unchecked("3")));

        let mut class_hashes = HashMap::new();
        class_hashes
            .insert(Felt::from_hex_unchecked("1").try_into().unwrap(), ClassHash(Felt::from_hex_unchecked("0xc1a551")));
        class_hashes
            .insert(Felt::from_hex_unchecked("2").try_into().unwrap(), ClassHash(Felt::from_hex_unchecked("0xc1a552")));
        class_hashes
            .insert(Felt::from_hex_unchecked("3").try_into().unwrap(), ClassHash(Felt::from_hex_unchecked("0xc1a553")));

        let mut storage = HashMap::new();
        storage.insert(
            (Felt::from_hex_unchecked("1").try_into().unwrap(), Felt::from_hex_unchecked("1").try_into().unwrap()),
            Felt::from_hex_unchecked("1"),
        );
        storage.insert(
            (Felt::from_hex_unchecked("1").try_into().unwrap(), Felt::from_hex_unchecked("2").try_into().unwrap()),
            Felt::from_hex_unchecked("2"),
        );
        storage.insert(
            (Felt::from_hex_unchecked("1").try_into().unwrap(), Felt::from_hex_unchecked("3").try_into().unwrap()),
            Felt::from_hex_unchecked("3"),
        );

        storage.insert(
            (Felt::from_hex_unchecked("2").try_into().unwrap(), Felt::from_hex_unchecked("1").try_into().unwrap()),
            Felt::from_hex_unchecked("1"),
        );
        storage.insert(
            (Felt::from_hex_unchecked("2").try_into().unwrap(), Felt::from_hex_unchecked("2").try_into().unwrap()),
            Felt::from_hex_unchecked("2"),
        );
        storage.insert(
            (Felt::from_hex_unchecked("2").try_into().unwrap(), Felt::from_hex_unchecked("3").try_into().unwrap()),
            Felt::from_hex_unchecked("3"),
        );

        storage.insert(
            (Felt::from_hex_unchecked("3").try_into().unwrap(), Felt::from_hex_unchecked("1").try_into().unwrap()),
            Felt::from_hex_unchecked("1"),
        );
        storage.insert(
            (Felt::from_hex_unchecked("3").try_into().unwrap(), Felt::from_hex_unchecked("2").try_into().unwrap()),
            Felt::from_hex_unchecked("2"),
        );
        storage.insert(
            (Felt::from_hex_unchecked("3").try_into().unwrap(), Felt::from_hex_unchecked("3").try_into().unwrap()),
            Felt::from_hex_unchecked("3"),
        );

        let mut compiled_class_hashes = HashMap::new();
        // "0xc1a553" is marked as deprecated by not having a compiled
        // class hashe
        compiled_class_hashes.insert(
            ClassHash(Felt::from_hex_unchecked("0xc1a551")),
            CompiledClassHash(Felt::from_hex_unchecked("0x1")),
        );
        compiled_class_hashes.insert(
            ClassHash(Felt::from_hex_unchecked("0xc1a552")),
            CompiledClassHash(Felt::from_hex_unchecked("0x2")),
        );

        let mut declared_contracts = HashMap::new();
        declared_contracts.insert(ClassHash(Felt::from_hex_unchecked("0xc1a551")), true);
        declared_contracts.insert(ClassHash(Felt::from_hex_unchecked("0xc1a552")), true);
        declared_contracts.insert(ClassHash(Felt::from_hex_unchecked("0xc1a553")), true);

        let state_map = StateMaps { nonces, class_hashes, storage, compiled_class_hashes, declared_contracts };

        let storage_diffs = vec![
            ContractStorageDiffItem {
                address: Felt::from_hex_unchecked("1"),
                storage_entries: vec![
                    StorageEntry { key: Felt::from_hex_unchecked("1"), value: Felt::ONE },
                    StorageEntry { key: Felt::from_hex_unchecked("2"), value: Felt::TWO },
                    StorageEntry { key: Felt::from_hex_unchecked("3"), value: Felt::THREE },
                ],
            },
            ContractStorageDiffItem {
                address: Felt::from_hex_unchecked("2"),
                storage_entries: vec![
                    StorageEntry { key: Felt::from_hex_unchecked("1"), value: Felt::ONE },
                    StorageEntry { key: Felt::from_hex_unchecked("2"), value: Felt::TWO },
                    StorageEntry { key: Felt::from_hex_unchecked("3"), value: Felt::THREE },
                ],
            },
            ContractStorageDiffItem {
                address: Felt::from_hex_unchecked("3"),
                storage_entries: vec![
                    StorageEntry { key: Felt::from_hex_unchecked("1"), value: Felt::ONE },
                    StorageEntry { key: Felt::from_hex_unchecked("2"), value: Felt::TWO },
                    StorageEntry { key: Felt::from_hex_unchecked("3"), value: Felt::THREE },
                ],
            },
        ];

        let deprecated_declared_classes = vec![Felt::from_hex_unchecked("0xc1a553")];

        let declared_classes = vec![
            DeclaredClassItem {
                class_hash: Felt::from_hex_unchecked("0xc1a551"),
                compiled_class_hash: Felt::from_hex_unchecked("0x1"),
            },
            DeclaredClassItem {
                class_hash: Felt::from_hex_unchecked("0xc1a552"),
                compiled_class_hash: Felt::from_hex_unchecked("0x2"),
            },
        ];

        let nonces = vec![
            NonceUpdate { contract_address: Felt::from_hex_unchecked("1"), nonce: Felt::from_hex_unchecked("1") },
            NonceUpdate { contract_address: Felt::from_hex_unchecked("2"), nonce: Felt::from_hex_unchecked("2") },
            NonceUpdate { contract_address: Felt::from_hex_unchecked("3"), nonce: Felt::from_hex_unchecked("3") },
        ];

        let deployed_contracts = vec![
            DeployedContractItem {
                address: Felt::from_hex_unchecked("1"),
                class_hash: Felt::from_hex_unchecked("0xc1a551"),
            },
            DeployedContractItem {
                address: Felt::from_hex_unchecked("2"),
                class_hash: Felt::from_hex_unchecked("0xc1a552"),
            },
            DeployedContractItem {
                address: Felt::from_hex_unchecked("3"),
                class_hash: Felt::from_hex_unchecked("0xc1a553"),
            },
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

        let mut actual = super::create_normalized_state_diff(&backend, &Option::<_>::None, state_map).unwrap();

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
