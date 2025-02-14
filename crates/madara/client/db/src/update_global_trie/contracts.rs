use crate::db_block_id::RawDbBlockId;
use crate::MadaraBackend;
use crate::{bonsai_identifier, MadaraStorageError};
use bitvec::order::Msb0;
use bitvec::vec::BitVec;
use bitvec::view::AsBits;
use bonsai_trie::id::BasicId;
use mp_state_update::{ContractStorageDiffItem, DeployedContractItem, NonceUpdate, ReplacedClassItem, StorageEntry};
use rayon::prelude::*;
use starknet_types_core::felt::Felt;
use starknet_types_core::hash::{Pedersen, StarkHash};
use std::collections::HashMap;

#[derive(Debug, Default)]
struct ContractLeaf {
    pub class_hash: Option<Felt>,
    pub storage_root: Option<Felt>,
    pub nonce: Option<Felt>,
}

/// Calculates the contract trie root
///
/// # Arguments
///
/// * `csd`             - Commitment state diff for the current block.
/// * `block_number`    - The current block number.
///
/// # Returns
///
/// The contract root.
pub fn contract_trie_root(
    backend: &MadaraBackend,
    deployed_contracts: &[DeployedContractItem],
    replaced_classes: &[ReplacedClassItem],
    nonces: &[NonceUpdate],
    storage_diffs: &[ContractStorageDiffItem],
    block_number: u64,
) -> Result<Felt, MadaraStorageError> {
    let mut contract_leafs: HashMap<Felt, ContractLeaf> = HashMap::new();

    let mut contract_storage_trie = backend.contract_storage_trie();

    tracing::trace!("contract_storage_trie inserting");

    // First we insert the contract storage changes
    for ContractStorageDiffItem { address, storage_entries } in storage_diffs {
        for StorageEntry { key, value } in storage_entries {
            let bytes = key.to_bytes_be();
            let bv: BitVec<u8, Msb0> = bytes.as_bits()[5..].to_owned();
            contract_storage_trie.insert(&address.to_bytes_be(), &bv, value)?;
        }
        // insert the contract address in the contract_leafs to put the storage root later
        contract_leafs.insert(*address, Default::default());
    }

    tracing::trace!("contract_storage_trie commit");

    // Then we commit them
    contract_storage_trie.commit(BasicId::new(block_number))?;

    for NonceUpdate { contract_address, nonce } in nonces {
        contract_leafs.entry(*contract_address).or_default().nonce = Some(*nonce);
    }

    for DeployedContractItem { address, class_hash } in deployed_contracts {
        contract_leafs.entry(*address).or_default().class_hash = Some(*class_hash);
    }

    for ReplacedClassItem { contract_address, class_hash } in replaced_classes {
        contract_leafs.entry(*contract_address).or_default().class_hash = Some(*class_hash);
    }

    let mut contract_trie = backend.contract_trie();

    let leaf_hashes: Vec<_> = contract_leafs
        .into_par_iter()
        .map(|(contract_address, mut leaf)| {
            let storage_root = contract_storage_trie.root_hash(&contract_address.to_bytes_be())?;
            leaf.storage_root = Some(storage_root);
            let leaf_hash = contract_state_leaf_hash(backend, &contract_address, &leaf, block_number)?;
            let bytes = contract_address.to_bytes_be();
            let bv: BitVec<u8, Msb0> = bytes.as_bits()[5..].to_owned();
            Ok((bv, leaf_hash))
        })
        .collect::<Result<_, MadaraStorageError>>()?;

    for (k, v) in leaf_hashes {
        contract_trie.insert(bonsai_identifier::CONTRACT, &k, &v)?;
    }

    tracing::trace!("contract_trie committing");

    contract_trie.commit(BasicId::new(block_number))?;
    let root_hash = contract_trie.root_hash(bonsai_identifier::CONTRACT)?;

    tracing::trace!("contract_trie committed");

    Ok(root_hash)
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
    backend: &MadaraBackend,
    contract_address: &Felt,
    contract_leaf: &ContractLeaf,
    block_number: u64,
) -> Result<Felt, MadaraStorageError> {
    let nonce = contract_leaf.nonce.unwrap_or(
        backend.get_contract_nonce_at(&RawDbBlockId::Number(block_number), contract_address)?.unwrap_or(Felt::ZERO),
    );

    let class_hash = contract_leaf.class_hash.unwrap_or(
        backend.get_contract_class_hash_at(&RawDbBlockId::Number(block_number), contract_address)?.unwrap_or(Felt::ZERO), // .ok_or(MadaraStorageError::InconsistentStorage("Class hash not found".into()))?
    );

    let storage_root = contract_leaf
        .storage_root
        .ok_or(MadaraStorageError::InconsistentStorage("Storage root need to be set".into()))?;

    tracing::trace!("contract is {contract_address:#x} block_n={block_number} nonce={nonce:#x} class_hash={class_hash:#x} storage_root={storage_root:#x}");

    // computes the contract state leaf hash
    Ok(Pedersen::hash(&Pedersen::hash(&Pedersen::hash(&class_hash, &storage_root), &nonce), &Felt::ZERO))
}

#[cfg(test)]
mod contract_trie_root_tests {
    use mp_chain_config::ChainConfig;
    use crate::update_global_trie::tests::setup_test_backend;

    use super::*;
    use std::sync::Arc;
    use rstest::*;

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
        let result =
            contract_trie_root(&backend, &deployed_contracts, &replaced_classes, &nonces, &storage_diffs, block_number)
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
        let result = contract_state_leaf_hash(&backend, &contract_address, &contract_leaf, /* block_number */ 0).unwrap();
        assert_eq!(
            result,
            Felt::from_hex_unchecked("0x6bbd8d4b5692148f83c38e19091f64381b5239e2a73f53b59be3ec3efb41143")
        );
    }
}
