use starknet_types_core::{
    felt::Felt,
    hash::{Poseidon, StarkHash},
};

pub mod classes;
pub mod contracts;

/// "STARKNET_STATE_V0"
const STARKNET_STATE_PREFIX: Felt = Felt::from_hex_unchecked("0x535441524b4e45545f53544154455f5630");

pub fn calculate_state_root(contracts_trie_root: Felt, classes_trie_root: Felt) -> Felt {
    tracing::trace!("global state root calc {contracts_trie_root:#x} {classes_trie_root:#x}");
    if classes_trie_root == Felt::ZERO {
        contracts_trie_root
    } else {
        Poseidon::hash_array(&[STARKNET_STATE_PREFIX, contracts_trie_root, classes_trie_root])
    }
}
