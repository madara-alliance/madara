use serde::{Deserialize, Serialize};
use starknet_core::types::StorageProof;
use starknet_types_core::felt::Felt;

use crate::error::ProofVerificationError;
use crate::types::{PoseidonHash, TrieNode};

#[allow(dead_code)]
#[derive(Clone, Deserialize, Serialize)]
pub struct ClassProof {
    pub class_commitment: Felt,
    pub class_proof: Vec<TrieNode>,
}

// Implementations for ClassProof
impl ClassProof {
    /// Gets the "class_commitment" which is aka the root node of the class Merkle tree.
    ///
    /// Proof always starts with the root node, which means all we have to do is hash the
    /// first node in the proof to get the same thing.
    #[allow(clippy::result_large_err)]
    pub fn class_commitment(&self) -> Result<Felt, ProofVerificationError> {
        if !self.class_proof.is_empty() {
            let hash = self.class_proof[0].calculate_node_hash::<PoseidonHash>();
            Ok(hash)
        } else {
            Err(ProofVerificationError::EmptyProof)
        }
    }
}

impl From<StorageProof> for ClassProof {
    fn from(proof: StorageProof) -> Self {
        let class_proof = proof
            .classes_proof
            .iter()
            .map(|(node_hash, node)| {
                let mut trie_node: TrieNode = node.clone().into();
                // Set the node_hash from the NodeWithHash
                trie_node.set_node_hash(*node_hash);
                trie_node
            })
            .collect();
        let class_commitment = proof.global_roots.classes_tree_root;
        ClassProof { class_commitment, class_proof }
    }
}
