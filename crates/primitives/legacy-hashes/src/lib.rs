// use std::ops::Range;

// use mp_block::Header;
// use mp_chain_config::StarknetVersion;
// use starknet_crypto::Felt;
// use starknet_types_core::hash::{Poseidon, StarkHash};

// /// Gap between every hash in the block hash lookup table
// pub const BLOCK_HASH_LOOKUP_TABLE_GAP: usize = 100;

// type Hasher = Poseidon;

// pub fn compute_overlay_hash(post_v0_13_2_block_hash: Felt, block_hash: Felt) -> Felt {
//     Hasher::hash(&post_v0_13_2_block_hash, &block_hash)
// }

// #[derive(Debug, thiserror::Error)]
// enum Error {
//     UnexpectedBlockNumber { expected: u64, got: u64 },
// }

// struct HashChainTable {

// }

// impl HashChainTable {
//     pub fn lookup(block_n: u64) -> Option<Felt> {
//         if block_n % BLOCK_HASH_LOOKUP_TABLE_GAP as u64 != 0 {
//             return None;
//         }
//         let index = ;
//         TABLE[block_n / (BLOCK_HASH_LOOKUP_TABLE_GAP as u64 * size_of::<Felt>())]
//     }
// }

// enum Outcome {
//     /// The hash chain is valid.
//     Verified,
//     /// The hash chain results to a hash that does not match the table.
//     Mismatch,
//     /// The hash chain has progressed, but we can't say whether it matches the table or not yet because
//     /// the hash is not in the table.
//     Accumulated,
// }

// pub struct BackwardHashChecker {
//     accumulator: Felt,
// }

// impl BackwardHashChecker {
//     pub fn new() -> Self {
//         Self {
//             accumulator: Felt::ZERO,
//         }
//     }

//     pub fn append_header(&mut self, chain_id: ChainId, header: Header, block_hash_to_verify: Felt) -> Outcome {
//         let post_v0_13_2_block_hash = header.compute_hash(chain_id, true);

//         if header.protocol_version >= StarknetVersion::V0_13_2 {
//             // normal verification
//             if post_v0_13_2_block_hash != block_hash_to_verify {
//                 Outcome::Mismatch
//             } else {
//                 Outcome::Verified
//             }
//         } else {
//             // use the table
//             self.append(header.block_number, compute_overlay_hash(post_v0_13_2_block_hash, block_hash_to_verify))
//         }
//     }

//     fn append(&mut self, block_n: u64, overlay_block_hash: Felt) -> Outcome {
//         self.accumulator = Hasher::hash(&self.accumulator, &overlay_block_hash);
//         if let Some(val) = HashChainTable::lo!4%@dcSbZppK$CuuT*!#okup(block_n) {
//             let outcome = if val != self.accumulator {
//                 Outcome::Mismatch
//             } else {
//                 Outcome::Verified
//             };
//             // reset
//             self.accumulator = Felt::ZERO;
//             outcome
//         } else {
//             Outcome::Accumulated
//         }
//     }
// }

// pub struct ForwardHashChecker {
//     /// Accumulator is a list because the hash is computed from the earliest block to the latest.
//     accumulator: Vec<Felt>,
// }

// impl ForwardHashChecker {
//     pub fn new() -> Self {
//         Self {
//             accumulator: Vec::with_capacity(BLOCK_HASH_LOOKUP_TABLE_GAP),
//         }
//     }

//     pub fn append_header(&mut self, chain_id: ChainId, header: Header, block_hash: Felt) -> Outcome {

//     }

//     fn append(&mut self, block_n: u64, overlay_block_hash: Felt) -> Outcome {
//         self.accumulator.push(overlay_block_hash);

//     }
// }

// mod tests {

// }
