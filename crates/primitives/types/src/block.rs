use sp_core::H256;
use sp_runtime::generic::{Block, BlockId, Header};
use sp_runtime::traits::BlakeTwo256;
use sp_runtime::OpaqueExtrinsic;

// Deoxys block number type
pub type DBlockNumber = u32;

// Deoxys block header type
pub type DHeaderT = Header<DBlockNumber, BlakeTwo256>;

// Deoxys block type
pub type DBlockT = Block<DHeaderT, OpaqueExtrinsic>;

// Deoxys hash type
pub type DHashT = H256;

// Deoxys block identifier type
pub type DBlockID = BlockId<DBlockT>;
