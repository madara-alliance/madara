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
            || classes::class_trie_root(backend, &state_diff.declared_classes, block_n),
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

pub fn get_state_root(backend: &RocksDBStorage) ->  Result<Felt> {
    let contract_trie = backend.contract_trie();
    let contract_trie_root_hash = contract_trie.root_hash(bonsai_identifier::CONTRACT).unwrap();

    let class_trie = backend.class_trie();
    let class_trie_root_hash = class_trie.root_hash(bonsai_identifier::CLASS).unwrap();

    let state_root = calculate_state_root(contract_trie_root_hash, class_trie_root_hash);

    println!("contract_trie_root_hash: {contract_trie_root_hash:#?}");
    println!("class_trie_root_hash: {class_trie_root_hash:#?}");
    println!("state_root: {state_root:#?}");

    Ok(state_root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MadaraBackend;
    use mp_chain_config::ChainConfig;
    use rstest::*;
    use std::sync::Arc;

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
}
