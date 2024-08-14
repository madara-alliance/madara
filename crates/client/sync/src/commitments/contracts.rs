use std::collections::HashMap;

use bitvec::order::Msb0;
use bitvec::vec::BitVec;
use bitvec::view::AsBits;
use bonsai_trie::id::BasicId;
use dc_db::DeoxysBackend;
use dc_db::{bonsai_identifier, DeoxysStorageError};
use dp_block::{BlockId, BlockTag};
use dp_state_update::{ContractStorageDiffItem, DeployedContractItem, NonceUpdate, ReplacedClassItem, StorageEntry};
use starknet_types_core::felt::Felt;
use starknet_types_core::hash::{Pedersen, StarkHash};

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
    backend: &DeoxysBackend,
    deployed_contracts: &[DeployedContractItem],
    replaced_classes: &[ReplacedClassItem],
    nonces: &[NonceUpdate],
    storage_diffs: &[ContractStorageDiffItem],
    block_number: u64,
) -> Result<Felt, DeoxysStorageError> {
    let mut contract_leafs: HashMap<Felt, ContractLeaf> = HashMap::new();

    let mut contract_storage_trie = backend.contract_storage_trie();

    log::debug!("contract_storage_trie inserting");

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

    log::debug!("contract_storage_trie commit");

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

    for (contract_address, mut leaf) in contract_leafs {
        let storage_root = contract_storage_trie.root_hash(&contract_address.to_bytes_be())?;
        leaf.storage_root = Some(storage_root);
        // TODO: parrallelize this with rayon
        let leaf_hash = contract_state_leaf_hash(backend, &contract_address, &leaf)?;
        let bytes = contract_address.to_bytes_be();
        let bv: BitVec<u8, Msb0> = bytes.as_bits()[5..].to_owned();
        contract_trie.insert(bonsai_identifier::CONTRACT, &bv, &leaf_hash)?;
    }

    log::debug!("contract_trie committing");

    contract_trie.commit(BasicId::new(block_number))?;
    let root_hash = contract_trie.root_hash(bonsai_identifier::CONTRACT)?;

    log::debug!("contract_trie committed");

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
    backend: &DeoxysBackend,
    contract_address: &Felt,
    contract_leaf: &ContractLeaf,
) -> Result<Felt, DeoxysStorageError> {
    let nonce = contract_leaf.nonce.unwrap_or(
        backend.get_contract_nonce_at(&BlockId::Tag(BlockTag::Latest), contract_address)?.unwrap_or(Felt::ZERO),
    );

    let class_hash = contract_leaf.class_hash.unwrap_or(
        backend.get_contract_class_hash_at(&BlockId::Tag(BlockTag::Latest), contract_address)?.unwrap_or(Felt::ZERO), // .ok_or(DeoxysStorageError::InconsistentStorage("Class hash not found".into()))?
    );

    let storage_root = contract_leaf
        .storage_root
        .ok_or(DeoxysStorageError::InconsistentStorage("Storage root need to be set".into()))?;

    // computes the contract state leaf hash
    Ok(Pedersen::hash(&Pedersen::hash(&Pedersen::hash(&class_hash, &storage_root), &nonce), &Felt::ZERO))
}
