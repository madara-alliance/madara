use std::sync::Arc;

use bonsai_trie::bonsai_database::DatabaseKey;
use bonsai_trie::id::{BasicId, BasicIdBuilder};
use bonsai_trie::{BonsaiDatabase, BonsaiStorage, BonsaiStorageConfig, BonsaiTrieHash, Membership, ProofNode};
use mc_db::{Backend, DbError};
use mp_felt::Felt252Wrapper;
use mp_hashers::HasherT;
use mp_transactions::compute_hash::ComputeTransactionHash;
use mp_transactions::Transaction;
use parity_scale_codec::{Decode, Encode};
use sp_core::H256;
use sp_runtime::traits::Block as BlockT;
use starknet_ff::FieldElement;
use starknet_types_core::hash::Pedersen;

/// Compute the combined hash of the transaction hash and the signature.
///
/// Since the transaction hash doesn't take the signature values as its input
/// computing the transaction commitent uses a hash value that combines
/// the transaction hash with the array of signature values.
///
/// # Arguments
///
/// * `tx` - The transaction to compute the hash of.
///
/// # Returns
///
/// The transaction hash with signature.
pub fn calculate_transaction_hash_with_signature<H: HasherT>(
    tx: &Transaction,
    chain_id: Felt252Wrapper,
    block_number: u64,
) -> FieldElement
where
    H: HasherT,
{
    let include_signature = block_number >= 61394;

    let signature_hash = if matches!(tx, Transaction::Invoke(_)) || include_signature {
        // Include signatures for Invoke transactions or for all transactions
        // starting from block 61394
        H::compute_hash_on_elements(
            &tx.signature().iter().map(|elt| FieldElement::from(*elt)).collect::<Vec<FieldElement>>(),
        )
    } else {
        // Before block 61394, and for non-Invoke transactions, signatures are not included
        H::compute_hash_on_elements(&[])
    };

    let transaction_hashes =
        H::hash_elements(FieldElement::from(tx.compute_hash::<H>(chain_id, false, Some(block_number))), signature_hash);

    transaction_hashes
}

pub fn calculate_transaction_commitment<B, H>(
    transactions: &[Transaction],
    chain_id: Felt252Wrapper,
    block_number: u64,
    backend: &Arc<Backend<B>>,
) -> Result<Felt252Wrapper, DbError>
where
    B: BlockT,
    H: HasherT
{
    let config = BonsaiStorageConfig::default();
    let mut bonsai_db = *backend.bonsai().clone().as_ref();

    let mut batch = bonsai_db.create_batch();
    let mut bonsai_storage: BonsaiStorage<_, _, Pedersen> = BonsaiStorage::new(bonsai_db, config).unwrap();

    for (idx, tx) in transactions.iter().enumerate() {
        let idx: u64 = idx.try_into().expect("Too many transactions");
        let final_hash = calculate_transaction_hash_with_signature::<H>(tx, chain_id, block_number);
        bonsai_db.insert(&DatabaseKey::from(idx), &H256::from(final_hash.to_bytes_be()).encode(), Some(&mut batch));
    }

    bonsai_db.write_batch(batch);

    Ok(bonsai_storage.root_hash().expect("Failed to retrieve root hash from bonsai db").into())
}
