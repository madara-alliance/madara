use std::sync::Arc;

use bitvec::prelude::*;
use bonsai_trie::id::BasicIdBuilder;
use bonsai_trie::{BonsaiStorage, BonsaiStorageConfig};
use mc_db::bonsai_db::BonsaiDb;
use mp_felt::Felt252Wrapper;
use mp_hashers::HasherT;
use mp_transactions::compute_hash::ComputeTransactionHash;
use mp_transactions::Transaction;
use sp_runtime::traits::Block as BlockT;
use starknet_ff::FieldElement;
use starknet_types_core::felt::Felt;
use starknet_types_core::hash::Pedersen;
use tokio::task::{spawn_blocking, JoinSet};

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

pub(crate) async fn transaction_commitment<B, H>(
    transactions: &[Transaction],
    chain_id: Felt252Wrapper,
    block_number: u64,
    backend: Arc<BonsaiDb<B>>,
) -> Result<Felt252Wrapper, String>
where
    B: BlockT,
    H: HasherT,
{
    let config = BonsaiStorageConfig::default();
    let mut bonsai_storage = BonsaiStorage::<_, _, Pedersen>::new(backend.as_ref().clone(), config)
        .expect("Failed to create bonsai storage");

    let mut id_builder = BasicIdBuilder::new();

    let zero = id_builder.new_id();
    bonsai_storage.commit(zero).expect("Failed to commit to bonsai storage");

    let mut set = JoinSet::new();
    for (i, tx) in transactions.iter().cloned().enumerate() {
        let arc_tx = Arc::new(tx);
        set.spawn(async move { (i, calculate_transaction_hash_with_signature::<H>(&arc_tx, chain_id, block_number)) });
    }

    while let Some(res) = set.join_next().await {
        let (i, tx_hash) = res.map_err(|e| format!("Failed to compute transaction hash: {e}"))?;
        let key = BitVec::from_vec(i.to_be_bytes().to_vec());
        let value = Felt::from(Felt252Wrapper::from(tx_hash));
        bonsai_storage.insert(key.as_bitslice(), &value).expect("Failed to insert into bonsai storage");
    }

    let root_hash = spawn_blocking(move || {
        let id = id_builder.new_id();
        bonsai_storage.commit(id).expect("Failed to commit to bonsai storage");

        let root_hash = bonsai_storage.root_hash().expect("Failed to get root hash");
        bonsai_storage.revert_to(zero).unwrap();

        root_hash
    })
    .await
    .unwrap();

    Ok(Felt252Wrapper::from(root_hash))
}
