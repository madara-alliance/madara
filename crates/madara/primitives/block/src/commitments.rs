use bitvec::vec::BitVec;
use mp_chain_config::StarknetVersion;
use starknet_types_core::{
    felt::Felt,
    hash::{Pedersen, Poseidon, StarkHash},
};

pub fn compute_event_commitment(
    events_hashes: impl IntoIterator<Item = Felt>,
    starknet_version: StarknetVersion,
) -> Felt {
    let mut peekable = events_hashes.into_iter().peekable();
    if peekable.peek().is_none() {
        return Felt::ZERO;
    }
    if starknet_version < StarknetVersion::V0_13_2 {
        compute_merkle_root::<Pedersen>(peekable)
    } else {
        compute_merkle_root::<Poseidon>(peekable)
    }
}

pub fn compute_transaction_commitment(
    tx_hashes_with_signature: impl IntoIterator<Item = Felt>,
    starknet_version: StarknetVersion,
) -> Felt {
    if starknet_version < StarknetVersion::V0_13_2 {
        compute_merkle_root::<Pedersen>(tx_hashes_with_signature)
    } else {
        compute_merkle_root::<Poseidon>(tx_hashes_with_signature)
    }
}

pub fn compute_receipt_commitment(
    receipt_hashes: impl IntoIterator<Item = Felt>,
    _starknet_version: StarknetVersion,
) -> Felt {
    compute_merkle_root::<Poseidon>(receipt_hashes)
}

/// Compute the root hash of a list of values.
// The `HashMapDb` can't fail, so we can safely unwrap the results.
//
// perf: Note that committing changes still has the greatest performance hit
// as this is where the root hash is calculated. Due to the Merkle structure
// of Bonsai Tries, this results in a trie size that grows very rapidly with
// each new insertion. It seems that the only vector of optimization here
// would be to parallelize the tree traversal on insertion and optimize hash computation.
// It seems lambdaclass' crypto lib does not do simd hashing, we may want to look into that.
pub fn compute_merkle_root<H: StarkHash + Send + Sync>(values: impl IntoIterator<Item = Felt>) -> Felt {
    //TODO: replace the identifier by an empty slice when bonsai supports it
    const IDENTIFIER: &[u8] = b"0xinmemory";
    let config = bonsai_trie::BonsaiStorageConfig::default();
    let bonsai_db = bonsai_trie::databases::HashMapDb::<bonsai_trie::id::BasicId>::default();
    let mut bonsai_storage =
        bonsai_trie::BonsaiStorage::<_, _, H>::new(bonsai_db, config, /* max tree height */ 64);

    values.into_iter().enumerate().for_each(|(index, value)| {
        let key = BitVec::from_vec(index.to_be_bytes().to_vec()); // TODO: remove this useless allocation
        bonsai_storage.insert(IDENTIFIER, key.as_bitslice(), &value).expect("Failed to insert into bonsai storage");
    });

    let id = bonsai_trie::id::BasicIdBuilder::new().new_id();

    bonsai_storage.commit(id).expect("Failed to commit to bonsai storage");
    bonsai_storage.root_hash(IDENTIFIER).expect("Failed to get root hash")
}
