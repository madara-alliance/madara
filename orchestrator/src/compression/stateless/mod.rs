mod bitops;
mod buckets;
mod compress;
mod constants;
mod utils;

pub use compress::compress;

// NOTE: This code is mostly taken from Starkware's SNOS implementation. Here's the link for reference:
// https://github.com/starkware-libs/sequencer/blob/3b978f202e92714f07710c23d7d259ea6ca2f9e2/crates/starknet_os/src/hints/hint_implementation/stateless_compression/utils.rs
