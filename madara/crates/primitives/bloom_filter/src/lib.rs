//! Bloom filter primitives used for fast probabilistic membership checks
//! (e.g. event keys) over the node's stored data.
#![warn(missing_docs)]

mod filter;
mod storage;

pub use filter::{BloomFilter, PreCalculatedHashes};
pub use storage::{AtomicBitStore, BitStore};

#[cfg(test)]
mod tests;
