use crate::rocksdb::trie::WrappedBonsaiError;
use crate::{prelude::*, rocksdb::RocksDBStorage};
use mp_state_update::StateDiff;
use starknet_types_core::{
    felt::Felt,
    hash::{Poseidon, StarkHash},
};

mod classes;
mod contracts;

pub mod bonsai_identifier {
    pub const CONTRACT: &[u8] = b"0xcontract";
    pub const CLASS: &[u8] = b"0xclass";
}

/// Update the global tries.
/// Returns the new global state root. Multiple state diffs can be applied at once, only the latest state root will
/// be returned.
/// Errors if the batch is empty.
pub fn apply_to_global_trie<'a>(
    backend: &RocksDBStorage,
    start_block_n: u64,
    state_diffs: impl IntoIterator<Item = &'a StateDiff>,
) -> Result<Felt> {
    let mut state_root = None;
    for (block_n, state_diff) in (start_block_n..).zip(state_diffs) {
        tracing::debug!("applying state_diff block_n={block_n}");

        let (contract_trie_root, class_trie_root) = rayon::join(
            || {
                contracts::contract_trie_root(
                    backend,
                    &state_diff.deployed_contracts,
                    &state_diff.replaced_classes,
                    &state_diff.nonces,
                    &state_diff.storage_diffs,
                    block_n,
                )
            },
            || {
                classes::class_trie_root(
                    backend,
                    &state_diff.declared_classes,
                    &state_diff.migrated_compiled_classes,
                    block_n,
                )
            },
        );

        state_root = Some(calculate_state_root(contract_trie_root?, class_trie_root?));
    }
    state_root.context("Applying an empty batch to the global trie")
}

/// "STARKNET_STATE_V0"
const STARKNET_STATE_PREFIX: Felt = Felt::from_hex_unchecked("0x535441524b4e45545f53544154455f5630");

fn calculate_state_root(contracts_trie_root: Felt, classes_trie_root: Felt) -> Felt {
    tracing::trace!("global state root calc {contracts_trie_root:#x} {classes_trie_root:#x}");
    if classes_trie_root == Felt::ZERO {
        contracts_trie_root
    } else {
        Poseidon::hash_array(&[STARKNET_STATE_PREFIX, contracts_trie_root, classes_trie_root])
    }
}

pub fn get_state_root(backend: &RocksDBStorage) -> Result<Felt> {
    let contract_trie = backend.contract_trie();
    let contract_trie_root_hash = contract_trie.root_hash(bonsai_identifier::CONTRACT).map_err(WrappedBonsaiError)?;

    let class_trie = backend.class_trie();
    let class_trie_root_hash = class_trie.root_hash(bonsai_identifier::CLASS).map_err(WrappedBonsaiError)?;

    let state_root = calculate_state_root(contract_trie_root_hash, class_trie_root_hash);

    Ok(state_root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{rocksdb::RocksDBConfig, MadaraBackend, MadaraBackendConfig};
    use mc_class_exec::config::NativeConfig;
    use mc_class_exec::init_compilation_semaphore;
    use mp_chain_config::ChainConfig;
    use mp_state_update::{
        ContractStorageDiffItem, DeclaredClassItem, DeployedContractItem, MigratedClassItem, NonceUpdate,
        ReplacedClassItem,
    };
    use serde::Deserialize;
    use rstest::*;
    use std::collections::HashMap;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    #[derive(Debug, Deserialize)]
    struct Fixture {
        block_range: [u64; 2],
        blocks: Vec<FixtureBlock>,
    }

    #[derive(Debug, Deserialize)]
    struct FixtureBlock {
        block_number: u64,
        block_hash: Felt,
        old_root: Felt,
        new_root: Felt,
        state_diff: RpcStateDiff,
    }

    #[derive(Clone, Debug, Deserialize)]
    struct RpcStateDiff {
        declared_classes: Vec<DeclaredClassItem>,
        deployed_contracts: Vec<DeployedContractItem>,
        #[serde(default, rename = "deprecated_declared_classes")]
        old_declared_contracts: Vec<Felt>,
        nonces: Vec<NonceUpdate>,
        replaced_classes: Vec<ReplacedClassItem>,
        storage_diffs: Vec<ContractStorageDiffItem>,
        #[serde(default)]
        migrated_compiled_classes: Vec<MigratedClassItem>,
    }

    impl From<RpcStateDiff> for StateDiff {
        fn from(value: RpcStateDiff) -> Self {
            Self {
                storage_diffs: value.storage_diffs,
                old_declared_contracts: value.old_declared_contracts,
                declared_classes: value.declared_classes,
                deployed_contracts: value.deployed_contracts,
                replaced_classes: value.replaced_classes,
                nonces: value.nonces,
                migrated_compiled_classes: value.migrated_compiled_classes,
            }
        }
    }

    #[fixture]
    pub fn setup_test_backend() -> Arc<MadaraBackend> {
        let chain_config = Arc::new(ChainConfig::madara_test());
        MadaraBackend::open_for_testing(chain_config.clone())
    }

    /// Test cases for the `calculate_state_root` function.
    ///
    /// This test uses `rstest` to parameterize different scenarios for calculating
    /// the state root. It verifies that the function correctly handles various
    /// input combinations and produces the expected results.
    #[rstest]
    #[case::non_zero_inputs(
        Felt::from_hex_unchecked("0x123456"),  // Non-zero contracts trie root
        Felt::from_hex_unchecked("0x789abc"),  // Non-zero classes trie root
        // Expected result: Poseidon hash of STARKNET_STATE_PREFIX and both non-zero roots
        Felt::from_hex_unchecked("0x6beb971880d4b4996b10fe613b8d49fa3dda8f8b63156c919077e08c534d06e")
    )]
    #[case::zero_class_trie_root(
        Felt::from_hex_unchecked("0x123456"),  // Non-zero contracts trie root
        Felt::from_hex_unchecked("0x0"),       // Zero classes trie root
        Felt::from_hex_unchecked("0x123456")   // Expected result: same as contracts trie root
    )]
    fn test_calculate_state_root(
        #[case] contracts_trie_root: Felt,
        #[case] classes_trie_root: Felt,
        #[case] expected_result: Felt,
    ) {
        // GIVEN: We have a contracts trie root and a classes trie root

        // WHEN: We calculate the state root using these inputs
        let result = calculate_state_root(contracts_trie_root, classes_trie_root);

        // THEN: The calculated state root should match the expected result
        assert_eq!(result, expected_result, "State root should match the expected result");
    }

    #[test]
    fn test_apply_to_global_trie_from_fixture() {
        let fixture = load_fixture();
        let start_block = fixture_start_block(&fixture);
        let backend = match open_fixture_backend() {
            Some(backend) => backend,
            None => return,
        };

        let latest_confirmed = backend.latest_confirmed_block_n().unwrap_or(0);
        if latest_confirmed > start_block {
            eprintln!(
                "skipping fixture test: base db confirmed tip {} is ahead of start_block {}",
                latest_confirmed, start_block
            );
            return;
        }

        let mut fixture = fixture;

        fixture.blocks.sort_by_key(|block| block.block_number);
        assert!(!fixture.blocks.is_empty(), "fixture has no blocks");

        let mut last_root: Option<Felt> = None;
        for block in fixture.blocks {
            if let Some(prev_root) = last_root {
                assert_eq!(
                    block.old_root, prev_root,
                    "old_root mismatch at block {}",
                    block.block_number
                );
            }

            let state_diff: StateDiff = block.state_diff.into();
            let new_root = backend
                .write_access()
                .apply_to_global_trie(block.block_number, [&state_diff])
                .expect("apply_to_global_trie should succeed");

            assert_eq!(
                new_root, block.new_root,
                "new_root mismatch at block {} (hash {})",
                block.block_number, block.block_hash
            );
            last_root = Some(block.new_root);
        }
    }

    #[test]
    fn test_parallel_apply_to_global_trie_from_fixture() {
        let fixture = load_fixture();
        let start_block = fixture_start_block(&fixture);
        let backend = match open_fixture_backend() {
            Some(backend) => backend,
            None => return,
        };
        let latest_confirmed = backend.latest_confirmed_block_n().unwrap_or(0);
        if latest_confirmed > start_block {
            eprintln!(
                "skipping fixture parallel test: base db confirmed tip {} is ahead of start_block {}",
                latest_confirmed, start_block
            );
            return;
        }

        let copies = 5usize;
        let base_path = fixture_db_path();
        let temp_dir = tempfile::TempDir::with_prefix("madara-trie-poc-par").expect("temp dir");
        let mut copy_paths = Vec::with_capacity(copies);
        for i in 0..copies {
            let copy_path = temp_dir.path().join(format!("copy_{i}"));
            copy_dir_recursive(&base_path, &copy_path).expect("copy base db");
            copy_paths.push(copy_path);
        }

        let mut blocks = fixture.blocks;
        blocks.sort_by_key(|block| block.block_number);
        let diffs: Vec<StateDiff> = blocks.iter().map(|b| b.state_diff.clone().into()).collect();

        let mut expected_roots = HashMap::new();
        for block in &blocks {
            expected_roots.insert(block.block_number, block.new_root);
        }

        let mut assignments: Vec<Vec<usize>> = vec![Vec::new(); copies];
        for idx in 0..diffs.len() {
            assignments[idx % copies].push(idx);
        }

        let diffs = Arc::new(diffs);
        let results = Arc::new(std::sync::Mutex::new(Vec::new()));
        rayon::scope(|scope| {
            for (copy_idx, copy_path) in copy_paths.iter().enumerate() {
                let assigned = assignments[copy_idx].clone();
                let diffs = Arc::clone(&diffs);
                let results = Arc::clone(&results);
                scope.spawn(move |_| {
                    if assigned.is_empty() {
                        return;
                    }
                    let backend = open_backend_from_path(copy_path);
                    let mut current_index: isize = -1;
                    for target_index in assigned {
                        let block_number = start_block + target_index as u64 + 1;
                        let from = (current_index + 1) as usize;
                        let to = target_index;
                        let mut batch: Vec<StateDiff> = diffs[from..=to].iter().cloned().collect();
                        for diff in &mut batch {
                            diff.sort();
                        }
                        let root = backend
                            .write_access()
                            .apply_to_global_trie(start_block + from as u64 + 1, batch.iter())
                            .expect("apply_to_global_trie");
                        results.lock().unwrap().push((block_number, root));
                        current_index = target_index as isize;
                    }
                });
            }
        });

        let results = results.lock().unwrap();
        for (block_number, root) in results.iter() {
            let expected = expected_roots.get(block_number).expect("expected root");
            assert_eq!(root, expected, "parallel root mismatch at block {}", block_number);
        }
    }

    fn fixture_db_path() -> PathBuf {
        std::env::var("MADARA_POC_DB_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/tmp/madara_devnet_poc_v0"))
    }

    fn open_fixture_backend() -> Option<Arc<MadaraBackend>> {
        let db_path = fixture_db_path();
        if !db_path.exists() {
            eprintln!("skipping fixture tests: db path {} not found", db_path.display());
            return None;
        }
        Some(open_backend_from_path(&db_path))
    }

    fn open_backend_from_path(db_path: &Path) -> Arc<MadaraBackend> {
        let chain_config = Arc::new(chain_config_for_fixture());
        let rocksdb_config = RocksDBConfig::default();
        let backend_config = MadaraBackendConfig::default();

        let native_builder = NativeConfig::builder();
        let max_concurrent = native_builder.max_concurrent_compilations();
        init_compilation_semaphore(max_concurrent);
        let cairo_native_config = Arc::new(native_builder.build());

        MadaraBackend::open_rocksdb(db_path, chain_config, backend_config, rocksdb_config, cairo_native_config)
            .expect("open rocksdb")
    }

    fn chain_config_for_fixture() -> ChainConfig {
        let mut config = ChainConfig::madara_devnet();
        let chain_id = std::env::var("MADARA_POC_CHAIN_ID").unwrap_or_else(|_| "POC_DEVNET".into());
        config.chain_id = starknet_api::core::ChainId::Other(chain_id.into());
        config
    }

    fn load_fixture() -> Fixture {
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../..");
        let fixture_path = repo_root.join("docs/poc_state_updates.json");
        let fixture_raw = fs::read_to_string(&fixture_path)
            .unwrap_or_else(|err| panic!("failed to read fixture at {}: {err}", fixture_path.display()));
        serde_json::from_str::<Fixture>(&fixture_raw).expect("fixture json should parse")
    }

    fn fixture_start_block(fixture: &Fixture) -> u64 {
        fixture.block_range[0].saturating_sub(1)
    }

    fn copy_dir_recursive(src: &Path, dst: &Path) -> anyhow::Result<()> {
        if dst.exists() {
            fs::remove_dir_all(dst).context("removing existing copy")?;
        }
        fs::create_dir_all(dst).context("creating copy dir")?;
        for entry in fs::read_dir(src).context("read_dir failed")? {
            let entry = entry.context("read_dir entry failed")?;
            let src_path = entry.path();
            let dst_path = dst.join(entry.file_name());
            let file_type = entry.file_type().context("file_type failed")?;
            if file_type.is_dir() {
                fs::create_dir_all(&dst_path).context("create_dir_all failed")?;
                copy_dir_recursive(&src_path, &dst_path)?;
            } else if file_type.is_file() {
                fs::copy(&src_path, &dst_path).with_context(|| format!("copy failed for {}", src_path.display()))?;
            }
        }
        Ok(())
    }

}
