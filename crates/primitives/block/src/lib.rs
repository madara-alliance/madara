//! Starknet block primitives.

mod header;
mod ordered_events;
pub use header::Header;
pub use ordered_events::*;
use starknet_api::block::BlockHash;
use starknet_api::transaction::{Transaction, TransactionHash};

pub use primitive_types::{H160, U256};
use starknet_types_core::felt::Felt;

/// Block tag.
///
/// A tag specifying a dynamic reference to a block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BlockTag {
    Latest,
    Pending,
}

impl From<starknet_core::types::BlockTag> for BlockTag {
    fn from(value: starknet_core::types::BlockTag) -> Self {
        match value {
            starknet_core::types::BlockTag::Latest => BlockTag::Latest,
            starknet_core::types::BlockTag::Pending => BlockTag::Pending,
        }
    }
}
impl From<BlockTag> for starknet_core::types::BlockTag {
    fn from(value: BlockTag) -> Self {
        match value {
            BlockTag::Latest => starknet_core::types::BlockTag::Latest,
            BlockTag::Pending => starknet_core::types::BlockTag::Pending,
        }
    }
}

/// Block Id
/// Block hash, number or tag
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockId {
    Hash(Felt),
    Number(u64),
    Tag(BlockTag),
}

impl From<starknet_core::types::BlockId> for BlockId {
    fn from(value: starknet_core::types::BlockId) -> Self {
        match value {
            starknet_core::types::BlockId::Hash(felt) => BlockId::Hash(Felt::from_bytes_be(&felt.to_bytes_be())),
            starknet_core::types::BlockId::Number(number) => BlockId::Number(number),
            starknet_core::types::BlockId::Tag(tag) => BlockId::Tag(tag.into()),
        }
    }
}
impl From<BlockId> for starknet_core::types::BlockId {
    fn from(value: BlockId) -> Self {
        match value {
            BlockId::Hash(felt) => starknet_core::types::BlockId::Hash(felt),
            BlockId::Number(number) => starknet_core::types::BlockId::Number(number),
            BlockId::Tag(tag) => starknet_core::types::BlockId::Tag(tag.into()),
        }
    }
}

// Light version of the block with block_hash
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct DeoxysBlockInfo {
    header: Header,
    block_hash: BlockHash,
    tx_hashes: Vec<TransactionHash>,
}

impl DeoxysBlockInfo {
    pub fn new(header: Header, tx_hashes: Vec<TransactionHash>, block_hash: BlockHash) -> Self {
        Self { header, block_hash, tx_hashes }
    }

    pub fn header(&self) -> &Header {
        &self.header
    }
    pub fn tx_hashes(&self) -> &[TransactionHash] {
        &self.tx_hashes
    }
    pub fn block_hash(&self) -> &BlockHash {
        &self.block_hash
    }
    pub fn block_n(&self) -> u64 {
        self.header.block_number
    }
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct DeoxysBlockInner {
    /// The block transactions.
    transactions: Vec<Transaction>, // Vec<starknet_api::transaction::Transaction>,
    /// The block events.
    events: Vec<OrderedEvents>,
}

impl DeoxysBlockInner {
    pub fn new(transactions: Vec<Transaction>, events: Vec<OrderedEvents>) -> Self {
        Self { transactions, events }
    }

    pub fn transactions(&self) -> &[Transaction] {
        &self.transactions
    }
    pub fn events(&self) -> &[OrderedEvents] {
        &self.events
    }
}

/// Starknet block definition.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct DeoxysBlock {
    info: DeoxysBlockInfo,
    inner: DeoxysBlockInner,
}

impl DeoxysBlock {
    /// Creates a new block.
    pub fn new(info: DeoxysBlockInfo, inner: DeoxysBlockInner) -> Self {
        Self { info, inner }
    }

    pub fn tx_hashes(&self) -> &[TransactionHash] {
        &self.info.tx_hashes
    }
    pub fn block_hash(&self) -> &BlockHash {
        &self.info.block_hash
    }
    pub fn block_n(&self) -> u64 {
        self.info.header.block_number
    }

    pub fn info(&self) -> &DeoxysBlockInfo {
        &self.info
    }
    pub fn header(&self) -> &Header {
        &self.info.header
    }

    pub fn inner(&self) -> &DeoxysBlockInner {
        &self.inner
    }
    pub fn transactions(&self) -> &[Transaction] {
        &self.inner.transactions
    }
    pub fn events(&self) -> &[OrderedEvents] {
        &self.inner.events
    }
}

#[cfg(test)]
mod tests;
