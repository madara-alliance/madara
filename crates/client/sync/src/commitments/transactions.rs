use dp_block::StarknetVersion;
use dp_transactions::Transaction;
use dp_transactions::LEGACY_BLOCK_NUMBER;
use dp_transactions::MAIN_CHAIN_ID;
use dp_transactions::V0_7_BLOCK_NUMBER;
use rayon::prelude::*;
use starknet_types_core::felt::Felt;
use starknet_types_core::hash::Poseidon;
use starknet_types_core::hash::{Pedersen, StarkHash};

use super::compute_root;

/// Compute the combined hash of the transaction hash and the signature.
///
/// Since the transaction hash doesn't take the signature values as its input
/// computing the transaction commitent uses a hash value that combines
/// the transaction hash with the array of signature values.
///
/// # Arguments
///
/// * `transaction` - The transaction to compute the hash of.
///
/// # Returns
///
/// The transaction hash with signature.
pub fn calculate_transaction_leaf_with_hash(
    transaction: &Transaction,
    chain_id: Felt,
    starknet_version: StarknetVersion,
    block_number: u64,
) -> (Felt, Felt) {
    let include_signature = starknet_version >= StarknetVersion::STARKNET_VERSION_0_11_1;
    let legacy = block_number < LEGACY_BLOCK_NUMBER && chain_id == MAIN_CHAIN_ID;
    let is_pre_v0_7 = block_number < V0_7_BLOCK_NUMBER && chain_id == MAIN_CHAIN_ID;

    let tx_hash = if is_pre_v0_7 {
        transaction.compute_hash_pre_v0_7(chain_id, false)
    } else {
        transaction.compute_hash(chain_id, false, legacy)
    };

    let leaf = match transaction {
        Transaction::Invoke(tx) => {
            // Include signatures for Invoke transactions or for all transactions
            if starknet_version < StarknetVersion::STARKNET_VERSION_0_13_2 {
                let signature_hash = tx.compute_hash_signature::<Pedersen>();
                Pedersen::hash(&tx_hash, &signature_hash)
            } else {
                let elements: Vec<Felt> = std::iter::once(tx_hash).chain(tx.signature().iter().copied()).collect();
                Poseidon::hash_array(&elements)
            }
        }
        Transaction::Declare(tx) => {
            if include_signature {
                if starknet_version < StarknetVersion::STARKNET_VERSION_0_13_2 {
                    let signature_hash = tx.compute_hash_signature::<Pedersen>();
                    Pedersen::hash(&tx_hash, &signature_hash)
                } else {
                    let elements: Vec<Felt> = std::iter::once(tx_hash).chain(tx.signature().iter().copied()).collect();
                    Poseidon::hash_array(&elements)
                }
            } else {
                let signature_hash = Pedersen::hash_array(&[]);
                Pedersen::hash(&tx_hash, &signature_hash)
            }
        }
        Transaction::DeployAccount(tx) => {
            if include_signature {
                if starknet_version < StarknetVersion::STARKNET_VERSION_0_13_2 {
                    let signature_hash = tx.compute_hash_signature::<Pedersen>();
                    Pedersen::hash(&tx_hash, &signature_hash)
                } else {
                    let elements: Vec<Felt> = std::iter::once(tx_hash).chain(tx.signature().iter().copied()).collect();
                    Poseidon::hash_array(&elements)
                }
            } else {
                let signature_hash = Pedersen::hash_array(&[]);
                Pedersen::hash(&tx_hash, &signature_hash)
            }
        }
        _ => {
            if starknet_version < StarknetVersion::STARKNET_VERSION_0_13_2 {
                let signature_hash = Pedersen::hash_array(&[]);
                Pedersen::hash(&tx_hash, &signature_hash)
            } else {
                Poseidon::hash_array(&[tx_hash, Felt::ZERO])
            }
        }
    };

    (leaf, tx_hash)
}

/// Calculate the transaction commitment in memory using HashMapDb (which is more efficient for this
/// usecase).
///
/// # Arguments
///
/// * `transactions` - The transactions of the block
/// * `chain_id` - The current chain id
/// * `block_number` - The current block number
///
/// # Returns
///
/// The transaction commitment as `Felt`.
pub fn memory_transaction_commitment(
    transactions: &[Transaction],
    chain_id: Felt,
    starknet_version: StarknetVersion,
    block_number: u64,
) -> (Felt, Vec<Felt>) {
    // TODO @cchudant refacto/optimise this function

    // transaction hashes are computed in parallel
    let (leafs, txs_hashes): (Vec<Felt>, Vec<Felt>) = transactions
        .par_iter()
        .map(|tx| calculate_transaction_leaf_with_hash(tx, chain_id, starknet_version, block_number))
        .unzip();

    // once transaction hashes have finished computing, they are inserted into the local Bonsai db
    let root = if starknet_version < StarknetVersion::STARKNET_VERSION_0_13_2 {
        compute_root::<Pedersen>(&leafs)
    } else {
        compute_root::<Poseidon>(&leafs)
    };

    (root, txs_hashes)
}
