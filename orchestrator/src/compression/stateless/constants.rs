use crate::compression::stateless::bitops::BitLength;
use strum::EnumCount;

pub(crate) const COMPRESSION_VERSION: u8 = 0;
pub(crate) const HEADER_ELM_N_BITS: usize = 20; // Max value ~1M
pub(crate) const HEADER_ELM_BOUND: u32 = 1 << HEADER_ELM_N_BITS;
pub(crate) const HEADER_LEN: usize = 1 + 1 + N_UNIQUE_BUCKETS + 1; // version, len, buckets, repeating_len
pub(crate) const N_UNIQUE_BUCKETS: usize = BitLength::COUNT;
pub(crate) const TOTAL_N_BUCKETS: usize = N_UNIQUE_BUCKETS + 1; // Includes repeating bucket
pub(crate) const MAX_N_BITS: usize = 251;
