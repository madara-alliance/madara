use super::ContractTrieTimings;
use crate::metrics::metrics;
use crate::rocksdb::trie::{GlobalTrie, SharedContractStorageTrie, WrappedBonsaiError};
use crate::{prelude::*, rocksdb::RocksDBStorage};
use bitvec::order::Msb0;
use bitvec::vec::BitVec;
use bitvec::view::AsBits;
use bonsai_trie::id::BasicId;
use mp_state_update::{ContractStorageDiffItem, DeployedContractItem, NonceUpdate, ReplacedClassItem, StorageEntry};
use rayon::prelude::*;
use starknet_types_core::felt::Felt;
use starknet_types_core::hash::{Pedersen, StarkHash};
use std::collections::HashMap;
use std::time::Instant;

#[derive(Debug, Default)]
struct ContractLeaf {
    pub class_hash: Option<Felt>,
    pub storage_root: Option<Felt>,
    pub nonce: Option<Felt>,
}

/// Holds uncommitted contract tries between staged root computation and final commit.
pub struct StagedContractTries {
    backend: RocksDBStorage,
    contract_storage_trie: SharedContractStorageTrie,
    cache_generation: u64,
    contract_trie: GlobalTrie<Pedersen>,
    root_timings: ContractTrieTimings,
    committed: bool,
}

impl Drop for StagedContractTries {
    fn drop(&mut self) {
        if !self.committed {
            tracing::debug!("dropping staged contract tries without commit, resetting cached contract storage trie");
            self.backend.reset_cached_contract_storage_trie_if_generation(self.cache_generation);
        }
    }
}

struct ResetCachedStorageTrieOnDrop {
    backend: RocksDBStorage,
    cache_generation: Option<u64>,
    armed: bool,
}

impl Drop for ResetCachedStorageTrieOnDrop {
    fn drop(&mut self) {
        if self.armed {
            tracing::debug!("staged contract trie computation aborted, resetting cached contract storage trie");
            if let Some(cache_generation) = self.cache_generation {
                self.backend.reset_cached_contract_storage_trie_if_generation(cache_generation);
            }
        }
    }
}

impl StagedContractTries {
    pub fn commit(mut self, block_number: u64) -> Result<ContractTrieTimings> {
        let mut timings = self.root_timings.clone();

        let storage_commit_start = Instant::now();
        let mut contract_storage_trie =
            self.contract_storage_trie.write().unwrap_or_else(|poisoned| poisoned.into_inner());
        anyhow::ensure!(
            self.contract_storage_trie.generation() == self.cache_generation,
            "cached contract storage trie generation changed before staged commit"
        );
        contract_storage_trie.commit(BasicId::new(block_number)).map_err(WrappedBonsaiError)?;
        timings.storage_commit = storage_commit_start.elapsed();
        let storage_commit_secs = timings.storage_commit.as_secs_f64();
        metrics().contract_storage_trie_commit_duration.record(storage_commit_secs, &[]);
        metrics().contract_storage_trie_commit_last.record(storage_commit_secs, &[]);
        drop(contract_storage_trie);

        let contract_commit_start = Instant::now();
        self.contract_trie.commit(BasicId::new(block_number)).map_err(WrappedBonsaiError)?;
        timings.trie_commit = contract_commit_start.elapsed();
        let contract_commit_secs = timings.trie_commit.as_secs_f64();
        metrics().contract_trie_commit_duration.record(contract_commit_secs, &[]);
        metrics().contract_trie_commit_last.record(contract_commit_secs, &[]);

        self.committed = true;
        Ok(timings)
    }
}

/// Calculates the contract trie root from staged (uncommitted) changes.
/// Returns the root hash and the staged tries that can be committed later.
pub fn contract_trie_root_staged(
    backend: &RocksDBStorage,
    deployed_contracts: &[DeployedContractItem],
    replaced_classes: &[ReplacedClassItem],
    nonces: &[NonceUpdate],
    storage_diffs: &[ContractStorageDiffItem],
    block_number: u64,
) -> Result<(Felt, StagedContractTries)> {
    let mut contract_leafs: HashMap<Felt, ContractLeaf> = HashMap::new();
    let mut timings = ContractTrieTimings::default();
    let cached_contract_storage_trie = backend.cached_contract_storage_trie();
    let mut reset_cached_trie =
        ResetCachedStorageTrieOnDrop { backend: backend.clone(), cache_generation: None, armed: true };
    let cache_generation;

    {
        let mut contract_storage_trie =
            cached_contract_storage_trie.write().unwrap_or_else(|poisoned| poisoned.into_inner());
        cache_generation = cached_contract_storage_trie.generation();
        reset_cached_trie.cache_generation = Some(cache_generation);

        tracing::debug!(
            touched_contracts = storage_diffs.len(),
            storage_diff_entries = storage_diffs.iter().map(|diff| diff.storage_entries.len()).sum::<usize>(),
            "contract_storage_trie using cached frontier",
        );

        let storage_insert_start = Instant::now();
        for ContractStorageDiffItem { address, storage_entries } in storage_diffs {
            let address_bytes = address.to_bytes_be();
            let entries = storage_entries.iter().map(|StorageEntry { key, value }| {
                let key_bytes = key.to_bytes_be();
                let bv: BitVec<u8, Msb0> = key_bytes.as_bits()[5..].to_owned();
                (bv, *value)
            });
            contract_storage_trie
                .insert_many_owned_assume_changed(&address_bytes, entries)
                .map_err(WrappedBonsaiError)?;
            contract_leafs.insert(*address, Default::default());
        }
        timings.storage_insert = storage_insert_start.elapsed();
    }

    for NonceUpdate { contract_address, nonce } in nonces {
        contract_leafs.entry(*contract_address).or_default().nonce = Some(*nonce);
    }

    for DeployedContractItem { address, class_hash } in deployed_contracts {
        contract_leafs.entry(*address).or_default().class_hash = Some(*class_hash);
    }

    for ReplacedClassItem { contract_address, class_hash } in replaced_classes {
        contract_leafs.entry(*contract_address).or_default().class_hash = Some(*class_hash);
    }

    {
        let storage_root_start = Instant::now();
        let contract_storage_trie =
            cached_contract_storage_trie.read().unwrap_or_else(|poisoned| poisoned.into_inner());

        let contract_addresses = contract_leafs.keys().copied().collect::<Vec<_>>();
        let storage_roots = contract_addresses
            .par_iter()
            .map(|contract_address| {
                let storage_root = contract_storage_trie
                    .root_hash_staged(&contract_address.to_bytes_be())
                    .map_err(WrappedBonsaiError)?;
                anyhow::Ok((*contract_address, storage_root))
            })
            .collect::<Result<Vec<_>>>()?;

        for (contract_address, storage_root) in storage_roots {
            contract_leafs.get_mut(&contract_address).expect("storage root computed for known contract").storage_root =
                Some(storage_root);
        }
        timings.storage_root = storage_root_start.elapsed();
    }

    let leaf_hash_start = Instant::now();
    let leaf_hashes: Vec<_> = contract_leafs
        .into_par_iter()
        .map(|(contract_address, leaf)| {
            let leaf_hash = contract_state_leaf_hash(backend, &contract_address, &leaf, block_number)?;
            let bytes = contract_address.to_bytes_be();
            let bv: BitVec<u8, Msb0> = bytes.as_bits()[5..].to_owned();
            anyhow::Ok((bytes, bv, leaf_hash))
        })
        .collect::<Result<_>>()?;
    timings.leaf_hash = leaf_hash_start.elapsed();

    let mut contract_trie = backend.contract_trie();

    let trie_insert_start = Instant::now();
    let mut leaf_hashes: Vec<([u8; 32], BitVec<u8, Msb0>, Felt)> = leaf_hashes;
    leaf_hashes.sort_unstable_by_key(|(bytes, _, _)| *bytes);
    contract_trie
        .insert_many_owned_assume_changed(
            super::bonsai_identifier::CONTRACT,
            leaf_hashes.into_iter().map(|(_, key, value)| (key, value)),
        )
        .map_err(WrappedBonsaiError)?;
    timings.trie_insert = trie_insert_start.elapsed();

    let trie_root_hash_start = Instant::now();
    let root_hash = contract_trie.root_hash_staged(super::bonsai_identifier::CONTRACT).map_err(WrappedBonsaiError)?;
    timings.trie_root_hash = trie_root_hash_start.elapsed();

    tracing::trace!("contract_trie staged root computed");

    reset_cached_trie.armed = false;
    Ok((
        root_hash,
        StagedContractTries {
            backend: backend.clone(),
            contract_storage_trie: cached_contract_storage_trie,
            cache_generation,
            contract_trie,
            root_timings: timings,
            committed: false,
        },
    ))
}

/// Calculates the contract trie root (single-phase: inserts + commits immediately).
/// Used by the sync path which does not need staged validation.
pub fn contract_trie_root(
    backend: &RocksDBStorage,
    deployed_contracts: &[DeployedContractItem],
    replaced_classes: &[ReplacedClassItem],
    nonces: &[NonceUpdate],
    storage_diffs: &[ContractStorageDiffItem],
    block_number: u64,
) -> Result<(Felt, ContractTrieTimings)> {
    let (root_hash, staged) =
        contract_trie_root_staged(backend, deployed_contracts, replaced_classes, nonces, storage_diffs, block_number)?;
    let timings = staged.commit(block_number)?;
    Ok((root_hash, timings))
}

/// Computes the contract state leaf hash
///
/// # Arguments
///
/// * `csd`             - Commitment state diff for the current block.
/// * `contract_address` - The contract address.
/// * `storage_root`     - The storage root of the contract.
///
/// # Returns
///
/// The contract state leaf hash.
fn contract_state_leaf_hash(
    backend: &RocksDBStorage,
    contract_address: &Felt,
    contract_leaf: &ContractLeaf,
    block_number: u64,
) -> Result<Felt> {
    let nonce = contract_leaf
        .nonce
        .unwrap_or(backend.inner.get_contract_nonce_at(block_number, contract_address)?.unwrap_or(Felt::ZERO));

    let class_hash = if let Some(class_hash) = contract_leaf.class_hash {
        class_hash
    } else {
        backend.inner.get_contract_class_hash_at(block_number, contract_address)?.unwrap_or(Felt::ZERO)
    };

    let storage_root = contract_leaf.storage_root.context("Storage root need to be set")?;

    tracing::trace!("contract is {contract_address:#x} block_n={block_number} nonce={nonce:#x} class_hash={class_hash:#x} storage_root={storage_root:#x}");

    // computes the contract state leaf hash
    Ok(Pedersen::hash(&Pedersen::hash(&Pedersen::hash(&class_hash, &storage_root), &nonce), &Felt::ZERO))
}

#[cfg(test)]
mod contract_trie_root_tests {
    use super::*;
    use crate::{rocksdb::global_trie::tests::setup_test_backend, test_utils::add_test_block, MadaraBackend};
    use mp_chain_config::ChainConfig;
    use rstest::*;
    use std::sync::{mpsc, Arc, Barrier};
    use std::thread;

    fn sample_contract_address() -> Felt {
        Felt::from_hex_unchecked("0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef")
    }

    fn sample_storage_diffs(value: Felt) -> Vec<ContractStorageDiffItem> {
        vec![ContractStorageDiffItem {
            address: sample_contract_address(),
            storage_entries: vec![StorageEntry {
                key: Felt::from_hex_unchecked("0x0000000000000000000000000000000000000000000000000000000000000001"),
                value,
            }],
        }]
    }

    fn cached_storage_root(backend: &Arc<MadaraBackend>, contract_address: Felt) -> Felt {
        let cached_trie = backend.db.cached_contract_storage_trie();
        let trie = cached_trie.read().unwrap_or_else(|poisoned| poisoned.into_inner());
        trie.root_hash_staged(&contract_address.to_bytes_be()).unwrap()
    }

    #[rstest]
    fn test_contract_trie_root_success(setup_test_backend: Arc<MadaraBackend>) {
        let backend = setup_test_backend;
        // Create dummy data
        let deployed_contracts = vec![DeployedContractItem {
            address: Felt::from_hex_unchecked("0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef"),
            class_hash: Felt::from_hex_unchecked("0xfedcba0987654321fedcba0987654321fedcba0987654321fedcba0987654321"),
        }];

        let replaced_classes = vec![ReplacedClassItem {
            contract_address: Felt::from_hex_unchecked(
                "0xabcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890",
            ),
            class_hash: Felt::from_hex_unchecked("0x1234567890abcdeffedcba09876543211234567890abcdeffedcba0987654321"),
        }];

        let nonces = vec![NonceUpdate {
            contract_address: Felt::from_hex_unchecked(
                "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef",
            ),
            nonce: Felt::from_hex_unchecked("0x0000000000000000000000000000000000000000000000000000000000000001"),
        }];

        let storage_diffs = vec![ContractStorageDiffItem {
            address: Felt::from_hex_unchecked("0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef"),
            storage_entries: vec![StorageEntry {
                key: Felt::from_hex_unchecked("0x0000000000000000000000000000000000000000000000000000000000000001"),
                value: Felt::from_hex_unchecked("0x0000000000000000000000000000000000000000000000000000000000000002"),
            }],
        }];

        let block_number = 1;

        // Call the function and print the result
        let (result, _timings) = contract_trie_root(
            &backend.db,
            &deployed_contracts,
            &replaced_classes,
            &nonces,
            &storage_diffs,
            block_number,
        )
        .unwrap();

        assert_eq!(
            result,
            Felt::from_hex_unchecked("0x59b89ceac43986727fb4a57bd9f74690b5b3b0e976e7af0b10213c3d4392ef2")
        );
    }

    #[test]
    fn test_contract_state_leaf_hash_success() {
        let chain_config = Arc::new(ChainConfig::madara_test());
        let backend = MadaraBackend::open_for_testing(chain_config.clone());

        // Create dummy data
        let contract_address =
            Felt::from_hex_unchecked("0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef");
        let contract_leaf = ContractLeaf {
            class_hash: Some(Felt::from_hex_unchecked(
                "0xfedcba0987654321fedcba0987654321fedcba0987654321fedcba0987654321",
            )),
            storage_root: Some(Felt::from_hex_unchecked(
                "0xabcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890",
            )),
            nonce: Some(Felt::from_hex_unchecked("0x0000000000000000000000000000000000000000000000000000000000000001")),
        };

        // Call the function and print the result
        let result =
            contract_state_leaf_hash(&backend.db, &contract_address, &contract_leaf, /* block_number */ 0).unwrap();
        assert_eq!(
            result,
            Felt::from_hex_unchecked("0x6bbd8d4b5692148f83c38e19091f64381b5239e2a73f53b59be3ec3efb41143")
        );
    }

    #[rstest]
    fn test_cached_contract_storage_trie_resets_when_staged_tries_are_dropped(setup_test_backend: Arc<MadaraBackend>) {
        let backend = setup_test_backend;
        let contract_address = sample_contract_address();

        let (_root_hash, staged) =
            contract_trie_root_staged(&backend.db, &[], &[], &[], &sample_storage_diffs(Felt::from(2u64)), 1).unwrap();

        assert_ne!(cached_storage_root(&backend, contract_address), Felt::ZERO);

        drop(staged);

        assert_eq!(cached_storage_root(&backend, contract_address), Felt::ZERO);
    }

    #[rstest]
    fn test_cached_contract_storage_trie_supports_consecutive_commits(setup_test_backend: Arc<MadaraBackend>) {
        let backend = setup_test_backend;
        let contract_address = sample_contract_address();

        let (_first_root, first_staged) =
            contract_trie_root_staged(&backend.db, &[], &[], &[], &sample_storage_diffs(Felt::from(2u64)), 1).unwrap();
        let staged_storage_root_round_one = cached_storage_root(&backend, contract_address);
        first_staged.commit(1).unwrap();

        let committed_storage_root_round_one =
            backend.db.contract_storage_trie().root_hash(&contract_address.to_bytes_be()).unwrap();
        assert_eq!(staged_storage_root_round_one, committed_storage_root_round_one);

        let (_second_root, second_staged) =
            contract_trie_root_staged(&backend.db, &[], &[], &[], &sample_storage_diffs(Felt::from(3u64)), 2).unwrap();
        let staged_storage_root_round_two = cached_storage_root(&backend, contract_address);
        assert_ne!(staged_storage_root_round_two, committed_storage_root_round_one);

        second_staged.commit(2).unwrap();

        let committed_storage_root_round_two =
            backend.db.contract_storage_trie().root_hash(&contract_address.to_bytes_be()).unwrap();
        assert_eq!(staged_storage_root_round_two, committed_storage_root_round_two);
    }

    #[rstest]
    fn test_revert_resets_cached_contract_storage_trie(setup_test_backend: Arc<MadaraBackend>) {
        let backend = setup_test_backend;
        let contract_address = sample_contract_address();

        let block_0_hash = add_test_block(&backend, 0, vec![]);
        add_test_block(&backend, 1, vec![]);

        let (_root_hash, staged) =
            contract_trie_root_staged(&backend.db, &[], &[], &[], &sample_storage_diffs(Felt::from(9u64)), 2).unwrap();
        assert_ne!(cached_storage_root(&backend, contract_address), Felt::ZERO);

        backend.revert_to(&block_0_hash).unwrap();

        assert_eq!(cached_storage_root(&backend, contract_address), Felt::ZERO);

        drop(staged);
    }

    #[rstest]
    fn test_stale_staged_drop_does_not_reset_newer_cache_generation(setup_test_backend: Arc<MadaraBackend>) {
        let backend = setup_test_backend;
        let contract_address = sample_contract_address();

        let (_stale_root, stale_staged) =
            contract_trie_root_staged(&backend.db, &[], &[], &[], &sample_storage_diffs(Felt::from(2u64)), 1).unwrap();

        backend.db.reset_cached_contract_storage_trie();

        let (_fresh_root, fresh_staged) =
            contract_trie_root_staged(&backend.db, &[], &[], &[], &sample_storage_diffs(Felt::from(3u64)), 2).unwrap();
        let fresh_cache_root = cached_storage_root(&backend, contract_address);

        drop(stale_staged);

        assert_eq!(cached_storage_root(&backend, contract_address), fresh_cache_root);

        drop(fresh_staged);
    }

    #[rstest]
    fn test_stale_staged_commit_fails_after_cache_reset(setup_test_backend: Arc<MadaraBackend>) {
        let backend = setup_test_backend;

        let (_root_hash, staged) =
            contract_trie_root_staged(&backend.db, &[], &[], &[], &sample_storage_diffs(Felt::from(2u64)), 1).unwrap();

        backend.db.reset_cached_contract_storage_trie();

        let err = staged.commit(1).unwrap_err();
        assert!(format!("{err:#}").contains("generation changed before staged commit"));
    }

    #[rstest]
    fn test_concurrent_stale_drop_does_not_reset_live_post_reset_cache(setup_test_backend: Arc<MadaraBackend>) {
        let backend = setup_test_backend;
        let contract_address = sample_contract_address();

        let (_stale_root, stale_staged) =
            contract_trie_root_staged(&backend.db, &[], &[], &[], &sample_storage_diffs(Felt::from(2u64)), 1).unwrap();

        let start_barrier = Arc::new(Barrier::new(2));
        let finish_barrier = Arc::new(Barrier::new(2));
        let (fresh_root_tx, fresh_root_rx) = mpsc::channel();
        let backend_for_worker = backend.clone();
        let start_barrier_for_worker = start_barrier.clone();
        let finish_barrier_for_worker = finish_barrier.clone();

        let worker = thread::spawn(move || {
            start_barrier_for_worker.wait();
            backend_for_worker.db.reset_cached_contract_storage_trie();

            let (_fresh_root, fresh_staged) = contract_trie_root_staged(
                &backend_for_worker.db,
                &[],
                &[],
                &[],
                &sample_storage_diffs(Felt::from(3u64)),
                2,
            )
            .unwrap();

            let fresh_cache_root = cached_storage_root(&backend_for_worker, contract_address);
            fresh_root_tx.send(fresh_cache_root).unwrap();

            finish_barrier_for_worker.wait();
            drop(fresh_staged);
        });

        start_barrier.wait();
        let fresh_cache_root = fresh_root_rx.recv().unwrap();
        drop(stale_staged);

        assert_eq!(cached_storage_root(&backend, contract_address), fresh_cache_root);

        finish_barrier.wait();
        worker.join().unwrap();
    }

    #[rstest]
    fn test_concurrent_stale_commit_fails_without_clobbering_live_post_reset_cache(
        setup_test_backend: Arc<MadaraBackend>,
    ) {
        let backend = setup_test_backend;
        let contract_address = sample_contract_address();

        let (_stale_root, stale_staged) =
            contract_trie_root_staged(&backend.db, &[], &[], &[], &sample_storage_diffs(Felt::from(2u64)), 1).unwrap();

        let start_barrier = Arc::new(Barrier::new(2));
        let finish_barrier = Arc::new(Barrier::new(2));
        let (fresh_root_tx, fresh_root_rx) = mpsc::channel();
        let backend_for_worker = backend.clone();
        let start_barrier_for_worker = start_barrier.clone();
        let finish_barrier_for_worker = finish_barrier.clone();

        let worker = thread::spawn(move || {
            start_barrier_for_worker.wait();
            backend_for_worker.db.reset_cached_contract_storage_trie();

            let (_fresh_root, fresh_staged) = contract_trie_root_staged(
                &backend_for_worker.db,
                &[],
                &[],
                &[],
                &sample_storage_diffs(Felt::from(3u64)),
                2,
            )
            .unwrap();

            let fresh_cache_root = cached_storage_root(&backend_for_worker, contract_address);
            fresh_root_tx.send(fresh_cache_root).unwrap();

            finish_barrier_for_worker.wait();
            drop(fresh_staged);
        });

        start_barrier.wait();
        let fresh_cache_root = fresh_root_rx.recv().unwrap();

        let err = stale_staged.commit(1).unwrap_err();
        assert!(format!("{err:#}").contains("generation changed before staged commit"));
        assert_eq!(cached_storage_root(&backend, contract_address), fresh_cache_root);

        finish_barrier.wait();
        worker.join().unwrap();
    }
}
