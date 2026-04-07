use anyhow::bail;
use log::info;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use starknet::providers::ProviderError;
use starknet_core::types::StorageProof;
use starknet_types_core::felt::Felt;
use std::time::Instant;

use crate::constants::DEFAULT_STORAGE_TREE_HEIGHT;
use crate::error::ProofVerificationError;
use crate::types::{PedersenHash, Proof, TrieNode};

/// Hex value of the string "STARKNET_STATE_V0"
const STARKNET_STATE_V0: Felt = Felt::from_hex_unchecked("0x535441524b4e45545f53544154455f5630");

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
pub struct ContractProof {
    /// Poseidon hash of a constant string, contracts tree root, classes tree root
    pub state_commitment: Option<Felt>,
    /// Contracts tree root
    pub contract_commitment: Felt,
    /// Classes tree root
    pub class_commitment: Option<Felt>,
    /// Contract proof
    pub contract_proof: Vec<TrieNode>,
    /// Contract storage data
    pub contract_data: Option<ContractData>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct ContractData {
    /// Root of the Contract storage tree
    pub root: Felt,
    /// The proofs associated with the queried storage values
    pub storage_proofs: Vec<Vec<TrieNode>>,
}

#[derive(Debug, Copy, Clone, PartialEq, Default, Eq, Hash, Serialize, Deserialize)]
pub struct Height(pub u64);

// Implementations for ContractData
impl ContractData {
    /// Verify the storage proofs and handle errors.
    /// Returns a list of additional keys to fetch to fill gaps in the tree that will make the OS
    /// crash otherwise.
    /// This function will panic if the proof contains an invalid node hash (i.e., the hash of a child
    /// node does not match the one specified in the parent).
    pub fn get_additional_keys(&self, keys: &[Felt]) -> anyhow::Result<Vec<Felt>> {
        info!("Fetching additional keys for a contract which already have {} keys", keys.len());
        let mut additional_keys = vec![];
        if let Err(errors) = self.verify(keys) {
            for error in errors {
                match error {
                    ProofVerificationError::NonExistenceProof { key, height, node } => {
                        if let TrieNode::Edge { child: _, path, .. } = &node {
                            if height.0 < DEFAULT_STORAGE_TREE_HEIGHT {
                                let modified_key = path.get_key_following_edge(key, height);
                                additional_keys.push(modified_key);
                            }
                        }
                    }
                    _ => {
                        bail!("Proof verification failed: {:?}", error);
                    }
                }
            }
        }

        Ok(additional_keys)
    }

    /// Verifies that each contract state proof is valid.
    pub fn verify(&self, storage_keys: &[Felt]) -> Result<(), Vec<ProofVerificationError>> {
        info!("Verifying contract data with {} storage keys", storage_keys.len());

        let proof_map = &self.storage_proofs[0].to_hashmap().map_err(|err| {
            vec![ProofVerificationError::ConversionError(format!("Failed to convert storage proofs: {}", err))]
        })?;

        let start = Instant::now();
        let errors: Vec<ProofVerificationError> = storage_keys
            .par_iter()
            .filter_map(|storage_key| proof_map.verify_proof::<PedersenHash>(*storage_key, self.root).err())
            .collect();

        let duration = start.elapsed();
        info!("Verification done in {:?}. Found {} errors", duration, errors.len());

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

impl TryFrom<StorageProof> for ContractProof {
    type Error = ProviderError;

    /// Convert [StorageProof] (returned as a response from RPC) to [ContractProof] (used by SNOS)
    ///
    /// TODO: Check if we can handle multiple contracts in this conversion function. Right now, it handles only a single contract from [StorageProof]
    /// TODO: Return proper errors
    fn try_from(proof: StorageProof) -> Result<Self, Self::Error> {
        // The `nodes` field in `contracts_proof` contains the union of all the paths from the
        // contracts-tree root to all the leaves
        let contract_proof = proof.contracts_proof;
        let contract_leaf = contract_proof.contract_leaves_data.first().ok_or(ProviderError::ArrayLengthMismatch)?;

        // Compute the state commitment
        let state_commitment = starknet_crypto::poseidon_hash_many(&[
            STARKNET_STATE_V0,
            proof.global_roots.contracts_tree_root,
            proof.global_roots.classes_tree_root,
        ]);

        // Convert storage proofs from response type to SNOS type
        let mut snos_storage_proofs: Vec<Vec<TrieNode>> = Vec::with_capacity(proof.contracts_storage_proofs.len());
        for contract_storage_proofs in proof.contracts_storage_proofs {
            snos_storage_proofs.push(
                contract_storage_proofs
                    .iter()
                    .map(|(node_hash, node)| {
                        // Convert the node from the response type to the SNOS type
                        let mut trie_node: TrieNode = node.clone().into();
                        trie_node.set_node_hash(*node_hash);
                        trie_node
                    })
                    .collect(),
            );
        }

        // Convert contract proofs from response to SNOS types
        // Here we are copying all the nodes from the response to the SNOS type
        // TODO: If we are processing only a single contract, we can optimize this by copying only the nodes for that contract
        let mut snos_contract_proof: Vec<TrieNode> = Vec::with_capacity(contract_proof.nodes.len());
        for (node_hash, node) in &contract_proof.nodes {
            // Convert the node from the response type to the SNOS type
            let mut trie_node: TrieNode = node.clone().into();
            // Set the node_hash from the NodeWithHash
            match &mut trie_node {
                TrieNode::Binary { node_hash: nh, .. } => *nh = Some(*node_hash),
                TrieNode::Edge { node_hash: nh, .. } => *nh = Some(*node_hash),
            }
            snos_contract_proof.push(trie_node);
        }

        Ok(ContractProof {
            state_commitment: Some(state_commitment),
            contract_commitment: proof.global_roots.contracts_tree_root,
            class_commitment: Some(proof.global_roots.classes_tree_root),
            contract_proof: snos_contract_proof,
            // TODO: Remove the unwrap. Make the root field Optional if possible or handle the error
            contract_data: Some(ContractData {
                root: contract_leaf.storage_root.unwrap(),
                storage_proofs: snos_storage_proofs,
            }),
        })
    }
}
