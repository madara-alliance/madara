use dp_receipt::TransactionReceipt;
use rayon::prelude::*;
use starknet_types_core::felt::Felt;
use starknet_types_core::hash::Poseidon;

use super::compute_root;

pub fn memory_receipt_commitment(receipts: &[TransactionReceipt]) -> Felt {
    let receipts_hash = receipts.par_iter().map(TransactionReceipt::compute_hash).collect::<Vec<_>>();
    compute_root::<Poseidon>(&receipts_hash)
}
