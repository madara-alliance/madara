use crate::metrics::metrics;
use crate::rocksdb::trie::WrappedBonsaiError;
use crate::{prelude::*, rocksdb::RocksDBStorage};
use mp_chain_config::StarknetVersion;
use mp_state_update::StateDiff;
use starknet_types_core::{
    felt::Felt,
    hash::{Poseidon, StarkHash},
};
use std::time::{Duration, Instant};

mod classes;
mod contracts;

/// Timing information from contract trie operations
#[derive(Debug, Clone, Default)]
pub struct ContractTrieTimings {
    /// Time to commit contract storage trie
    pub storage_commit: Duration,
    /// Time to commit contract trie
    pub trie_commit: Duration,
}

/// Timing information from class trie operations
#[derive(Debug, Clone, Default)]
pub struct ClassTrieTimings {
    /// Time to commit class trie
    pub trie_commit: Duration,
}

/// Timing information from global trie merklization
#[derive(Debug, Clone, Default)]
pub struct MerklizationTimings {
    /// Total time for merklization
    pub total: Duration,
    /// Time for contract trie root computation
    pub contract_trie_root: Duration,
    /// Time for class trie root computation
    pub class_trie_root: Duration,
    /// Sub-timings from contract trie
    pub contract_trie: ContractTrieTimings,
    /// Sub-timings from class trie
    pub class_trie: ClassTrieTimings,
}

pub use classes::StagedClassTrie;
pub use contracts::StagedContractTries;

pub mod bonsai_identifier {
    pub const CONTRACT: &[u8] = b"0xcontract";
    pub const CLASS: &[u8] = b"0xclass";
}

/// Holds all uncommitted global tries between staged root computation and final commit.
pub struct StagedGlobalTries {
    contract: StagedContractTries,
    class: StagedClassTrie,
    pub contract_trie_root_duration: Duration,
    pub class_trie_root_duration: Duration,
}

impl StagedGlobalTries {
    pub fn commit(self, block_number: u64) -> Result<(ContractTrieTimings, ClassTrieTimings)> {
        let contract_timings = self.contract.commit(block_number)?;
        let class_timings = self.class.commit(block_number)?;
        Ok((contract_timings, class_timings))
    }
}

/// Compute the global state root from staged (uncommitted) trie changes.
/// Nothing is persisted to disk. Call `StagedGlobalTries::commit()` to persist.
pub fn compute_global_trie_staged(
    backend: &RocksDBStorage,
    state_diff: &StateDiff,
    last_block_protocol_version: StarknetVersion,
    block_number: u64,
) -> Result<(Felt, StagedGlobalTries)> {
    let ((contract_result, contract_duration), (class_result, class_duration)) = rayon::join(
        || {
            let start = Instant::now();
            let result = contracts::contract_trie_root_staged(
                backend,
                &state_diff.deployed_contracts,
                &state_diff.replaced_classes,
                &state_diff.nonces,
                &state_diff.storage_diffs,
                block_number,
            );
            (result, start.elapsed())
        },
        || {
            let start = Instant::now();
            let result = classes::class_trie_root_staged(
                backend,
                &state_diff.declared_classes,
                &state_diff.migrated_compiled_classes,
            );
            (result, start.elapsed())
        },
    );

    let (contract_trie_root, staged_contracts) = contract_result?;
    let (class_trie_root, staged_classes) = class_result?;

    let state_root = calculate_state_root(contract_trie_root, class_trie_root, last_block_protocol_version);

    Ok((
        state_root,
        StagedGlobalTries {
            contract: staged_contracts,
            class: staged_classes,
            contract_trie_root_duration: contract_duration,
            class_trie_root_duration: class_duration,
        },
    ))
}

/// Update the global tries and compute the state root for the last block in the batch.
///
/// Applies each state diff to the contract and class tries in order. Only the state root of
/// the **last** block is computed and returned — intermediate trie states are committed (so the
/// bonsai trie is up to date) but their roots are not hashed into a state commitment.
///
/// # Arguments
///
/// * `last_block_protocol_version` — protocol version of the last block in the batch. Governs
///   whether the `class_trie_root == 0` short-circuit applies in [`calculate_state_root`]
///   (gated on `< 0.14.0`, matching pathfinder's `StateCommitment::calculate`).
///
/// # Errors
///
/// Returns an error if the batch is empty.
pub fn apply_to_global_trie<'a>(
    backend: &RocksDBStorage,
    start_block_n: u64,
    state_diffs: impl IntoIterator<Item = &'a StateDiff>,
    last_block_protocol_version: StarknetVersion,
) -> Result<(Felt, MerklizationTimings)> {
    let mut state_root = None;
    let mut timings = MerklizationTimings::default();

    for (block_n, state_diff) in (start_block_n..).zip(state_diffs) {
        tracing::debug!("applying state_diff block_n={block_n}");
        let block_start = Instant::now();

        let ((contract_result, contract_duration), (class_result, class_duration)) = rayon::join(
            || {
                let start = Instant::now();
                let result = contracts::contract_trie_root(
                    backend,
                    &state_diff.deployed_contracts,
                    &state_diff.replaced_classes,
                    &state_diff.nonces,
                    &state_diff.storage_diffs,
                    block_n,
                );
                (result, start.elapsed())
            },
            || {
                let start = Instant::now();
                let result = classes::class_trie_root(
                    backend,
                    &state_diff.declared_classes,
                    &state_diff.migrated_compiled_classes,
                    block_n,
                );
                (result, start.elapsed())
            },
        );

        // Record individual trie durations (histogram + gauge)
        let contract_secs = contract_duration.as_secs_f64();
        let class_secs = class_duration.as_secs_f64();
        metrics().contract_trie_root_duration.record(contract_secs, &[]);
        metrics().contract_trie_root_last.record(contract_secs, &[]);
        metrics().class_trie_root_duration.record(class_secs, &[]);
        metrics().class_trie_root_last.record(class_secs, &[]);

        // Extract root hashes and sub-timings
        let (contract_trie_root, contract_trie_timings) = contract_result?;
        let (class_trie_root, class_trie_timings) = class_result?;

        // NOTE: We compute the state root inside the loop (rather than only after it) so that
        // the per-block timing metrics below include the cost of `calculate_state_root`.
        //
        // Because we use `last_block_protocol_version` for every iteration, intermediate state
        // roots may be incorrect if the batch spans a protocol version boundary (e.g. 0.13.x →
        // 0.14.0). This is harmless today — only the final root is returned and verified — but
        // should be kept in mind if intermediate roots are ever used in the future.
        state_root = Some(calculate_state_root(contract_trie_root, class_trie_root, last_block_protocol_version));

        // Capture timings
        timings.contract_trie_root = contract_duration;
        timings.class_trie_root = class_duration;
        timings.contract_trie = contract_trie_timings;
        timings.class_trie = class_trie_timings;

        // Record total merklization duration per block (histogram + gauge)
        timings.total = block_start.elapsed();
        let block_secs = timings.total.as_secs_f64();
        metrics().apply_to_global_trie_duration.record(block_secs, &[]);
        metrics().apply_to_global_trie_last.record(block_secs, &[]);
    }

    let root = state_root.context("Applying an empty batch to the global trie")?;
    Ok((root, timings))
}

/// "STARKNET_STATE_V0"
const STARKNET_STATE_PREFIX: Felt = Felt::from_hex_unchecked("0x535441524b4e45545f53544154455f5630");

/// Computes the global state root from the contract and class trie roots.
///
/// Implements `calculate_global_state_root` from the Starknet OS
/// (`starkware/starknet/core/os/state/commitment.cairo` in the sequencer repo).
/// Pathfinder's `StateCommitment::calculate` (`crates/common/src/lib.rs`) is a
/// compatible independent implementation used as a cross-reference.
///
/// 1. If both roots are zero, the result is zero (any version).
/// 2. For `version < 0.14.0` with `class_trie_root == 0`, returns `contract_trie_root`
///    directly (legacy behavior — the class trie didn't exist yet).
/// 3. Otherwise (including all `>= 0.14.0` blocks, even when `class_trie_root == 0`),
///    the result is `Poseidon(STARKNET_STATE_V0, contract_trie_root, class_trie_root)`.
fn calculate_state_root(contracts_trie_root: Felt, classes_trie_root: Felt, protocol_version: StarknetVersion) -> Felt {
    tracing::trace!(
        "global state root calc contracts={contracts_trie_root:#x} classes={classes_trie_root:#x} version={protocol_version}"
    );

    if contracts_trie_root == Felt::ZERO && classes_trie_root == Felt::ZERO {
        return Felt::ZERO;
    }

    if classes_trie_root == Felt::ZERO && protocol_version < StarknetVersion::V0_14_0 {
        return contracts_trie_root;
    }

    Poseidon::hash_array(&[STARKNET_STATE_PREFIX, contracts_trie_root, classes_trie_root])
}

pub fn get_state_root(backend: &RocksDBStorage, protocol_version: StarknetVersion) -> Result<Felt> {
    let contract_trie = backend.contract_trie();
    let contract_trie_root_hash = contract_trie.root_hash(bonsai_identifier::CONTRACT).map_err(WrappedBonsaiError)?;

    let class_trie = backend.class_trie();
    let class_trie_root_hash = class_trie.root_hash(bonsai_identifier::CLASS).map_err(WrappedBonsaiError)?;

    let state_root = calculate_state_root(contract_trie_root_hash, class_trie_root_hash, protocol_version);

    Ok(state_root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MadaraBackend;
    use mp_chain_config::ChainConfig;
    use mp_state_update::{ContractStorageDiffItem, StorageEntry};
    use rstest::*;
    use std::sync::Arc;

    #[fixture]
    pub fn setup_test_backend() -> Arc<MadaraBackend> {
        let chain_config = Arc::new(ChainConfig::madara_test());
        MadaraBackend::open_for_testing(chain_config.clone())
    }

    /// Test cases for the `calculate_state_root` function.
    ///
    /// Cases 1-4 use version ≥ 0.14.0 (always hash unless both zero).
    /// Case 5 tests the pre-0.14.0 short-circuit (class_root == 0 → return contract_root).
    #[rstest]
    #[case::non_zero_inputs(
        Felt::from_hex_unchecked("0x123456"),
        Felt::from_hex_unchecked("0x789abc"),
        StarknetVersion::V0_14_1,
        // Poseidon(STARKNET_STATE_V0, 0x123456, 0x789abc)
        Felt::from_hex_unchecked("0x6beb971880d4b4996b10fe613b8d49fa3dda8f8b63156c919077e08c534d06e")
    )]
    // Regression test for Starknet ≥ 0.14.0: the `class_trie_root == 0` short-circuit must
    // NOT fire — the result is Poseidon(STARKNET_STATE_V0, contract_root, 0), not contract_root.
    #[case::zero_class_trie_root_post_0_14(
        Felt::from_hex_unchecked("0x3c538d437670f4c6f72dd799f215a007720ec7d19bc64195c96399145d8746f"),
        Felt::ZERO,
        StarknetVersion::V0_14_1,
        Felt::from_hex_unchecked("0x68bcf9e9257ab6bffd9425833a208aaab6b85649fd21c787a546cb7cb9abf")
    )]
    #[case::zero_contract_trie_root(
        Felt::ZERO,
        Felt::from_hex_unchecked("0x789abc"),
        StarknetVersion::V0_14_1,
        // Poseidon(STARKNET_STATE_V0, 0, 0x789abc)
        Felt::from_hex_unchecked("0x39eb9ca5f6c6dfdd4d91e3d1c03181b3c9bbc2bfe85e0d39b011bcd07cac79a")
    )]
    #[case::both_zero(Felt::ZERO, Felt::ZERO, StarknetVersion::V0_14_1, Felt::ZERO)]
    // Pre-0.14.0: class_root == 0 should short-circuit to contract_root (no hashing).
    #[case::zero_class_trie_root_pre_0_14(
        Felt::from_hex_unchecked("0x123456"),
        Felt::ZERO,
        StarknetVersion::V0_13_2,
        Felt::from_hex_unchecked("0x123456")  // contract_root returned directly
    )]
    fn test_calculate_state_root(
        #[case] contracts_trie_root: Felt,
        #[case] classes_trie_root: Felt,
        #[case] version: StarknetVersion,
        #[case] expected_result: Felt,
    ) {
        let result = calculate_state_root(contracts_trie_root, classes_trie_root, version);
        assert_eq!(result, expected_result, "State root should match the expected result");
    }

    /// End-to-end regression for the Starknet ≥ 0.14.0 state root bug.
    ///
    /// Drives a real Starknet 0.14.1 genesis-shaped state diff (a single storage write on the
    /// stateful-compression system contract `0x2`: `key=0x0 → value=0x80`) through
    /// `apply_to_global_trie` and asserts the returned global state root.
    ///
    /// This pins the full path: bonsai insert → `contract_trie_root` (with system-contract leaf
    /// hash: `class_hash=0, nonce=0`) → `calculate_state_root`. Against the old unconditional
    /// `class_trie_root == 0 → contract_root` short-circuit this test fails, because the
    /// short-circuit returns `0x3c538d…` (the contract trie root) instead of
    /// `Poseidon(STARKNET_STATE_V0, 0x3c538d…, 0) = 0x68bcf9…`.
    #[rstest]
    fn test_apply_to_global_trie_v0_14_genesis(setup_test_backend: Arc<MadaraBackend>) {
        let backend = setup_test_backend;

        let state_diff = StateDiff {
            storage_diffs: vec![ContractStorageDiffItem {
                address: Felt::from_hex_unchecked("0x2"),
                storage_entries: vec![StorageEntry { key: Felt::ZERO, value: Felt::from_hex_unchecked("0x80") }],
            }],
            ..Default::default()
        };

        let (state_root, _timings) = apply_to_global_trie(&backend.db, 0, [&state_diff], StarknetVersion::V0_14_1)
            .expect("apply_to_global_trie should succeed");

        assert_eq!(
            state_root,
            Felt::from_hex_unchecked("0x68bcf9e9257ab6bffd9425833a208aaab6b85649fd21c787a546cb7cb9abf"),
            "Global state root for 0.14.1 genesis state diff should equal Poseidon(STARKNET_STATE_V0, contract_root, 0)"
        );
    }
}
