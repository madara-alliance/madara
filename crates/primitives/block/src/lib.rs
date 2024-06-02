//! Starknet block primitives.
#![cfg_attr(not(feature = "std"), no_std)]

#[doc(hidden)]
pub extern crate alloc;
use alloc::vec::Vec;

mod header;
mod ordered_events;
pub use header::Header;
use mp_felt::Felt252Wrapper;
pub use ordered_events::*;
use starknet_api::block::BlockHash;
use starknet_api::transaction::{Transaction, TransactionHash};

/// Block Transactions
pub type BlockTransactions = Vec<Transaction>;

/// Block Events
pub type BlockEvents = Vec<OrderedEvents>;

/// Block tag.
///
/// A tag specifying a dynamic reference to a block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "parity-scale-codec", derive(parity_scale_codec::Encode, parity_scale_codec::Decode))]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[cfg_attr(feature = "scale-info", derive(scale_info::TypeInfo))]
pub enum BlockTag {
    #[cfg_attr(feature = "serde", serde(rename = "latest"))]
    Latest,
    #[cfg_attr(feature = "serde", serde(rename = "pending"))]
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
#[cfg_attr(feature = "parity-scale-codec", derive(parity_scale_codec::Encode, parity_scale_codec::Decode))]
#[cfg_attr(feature = "scale-info", derive(scale_info::TypeInfo))]
pub enum BlockId {
    Hash(Felt252Wrapper),
    Number(u64),
    Tag(BlockTag),
}

impl From<starknet_core::types::BlockId> for BlockId {
    fn from(value: starknet_core::types::BlockId) -> Self {
        match value {
            starknet_core::types::BlockId::Hash(felt) => BlockId::Hash(Felt252Wrapper(felt)),
            starknet_core::types::BlockId::Number(number) => BlockId::Number(number),
            starknet_core::types::BlockId::Tag(tag) => BlockId::Tag(tag.into()),
        }
    }
}
impl From<BlockId> for starknet_core::types::BlockId {
    fn from(value: BlockId) -> Self {
        match value {
            BlockId::Hash(felt) => starknet_core::types::BlockId::Hash(felt.0),
            BlockId::Number(number) => starknet_core::types::BlockId::Number(number),
            BlockId::Tag(tag) => starknet_core::types::BlockId::Tag(tag.into()),
        }
    }
}

// Light version of the block with block_hash
#[derive(Clone, Debug, Default)]
#[cfg_attr(feature = "parity-scale-codec", derive(parity_scale_codec::Encode, parity_scale_codec::Decode))]
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

#[derive(Clone, Debug, Default)]
#[cfg_attr(feature = "parity-scale-codec", derive(parity_scale_codec::Encode, parity_scale_codec::Decode))]
pub struct DeoxysBlockInner {
    /// The block transactions.
    transactions: BlockTransactions, // Vec<starknet_api::transaction::Transaction>,
    /// The block events.
    events: BlockEvents,
}

impl DeoxysBlockInner {
    pub fn new(transactions: BlockTransactions, events: BlockEvents) -> Self {
        Self { transactions, events }
    }

    pub fn transactions(&self) -> &BlockTransactions {
        &self.transactions
    }
    pub fn events(&self) -> &BlockEvents {
        &self.events
    }
}

/// Starknet block definition.
#[derive(Clone, Debug, Default)]
#[cfg_attr(feature = "parity-scale-codec", derive(parity_scale_codec::Encode, parity_scale_codec::Decode))]
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
    pub fn transactions(&self) -> &BlockTransactions {
        &self.inner.transactions
    }
    pub fn events(&self) -> &BlockEvents {
        &self.inner.events
    }
}

#[cfg(test)]
mod tests;
