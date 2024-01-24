use mp_felt::Felt252Wrapper;
use mp_hashers::HasherT;
use mp_transactions::compute_hash::ComputeTransactionHash;
use mp_transactions::Transaction;
use starknet_ff::FieldElement;

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

/// Calculate transaction commitment hash value.
///
/// The transaction commitment is the root of the Patricia Merkle tree with height 64
/// constructed by adding the (transaction_index, transaction_hash_with_signature)
/// key-value pairs to the tree and computing the root hash.
///
/// # Arguments
///
/// * `transactions` - The transactions to get the root from.
///
/// # Returns
///
/// The merkle root of the merkle tree built from the transactions.
pub fn calculate_transaction_commitment<H: HasherT>(
    transactions: &[Transaction],
    chain_id: Felt252Wrapper,
    block_number: u64,
) -> Felt252Wrapper {
    let mut tree = CommitmentTree::<H>::default();

    transactions.iter().enumerate().for_each(|(idx, tx)| {
        let idx: u64 = idx.try_into().expect("too many transactions while calculating commitment");
        let final_hash = calculate_transaction_hash_with_signature::<H>(tx, chain_id, block_number);
        tree.set(idx, final_hash);
    });
    tree.commit()
}