//! Starknet block primitives.

pub mod chain_config;
pub mod header;
mod starknet_version;

use dp_receipt::TransactionReceipt;
use dp_transactions::Transaction;
pub use header::Header;
use header::PendingHeader;
pub use primitive_types::{H160, U256};
use starknet_types_core::felt::Felt;
pub use starknet_version::{StarknetVersion, StarknetVersionError};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[allow(clippy::large_enum_variant)]
pub enum DeoxysMaybePendingBlockInfo {
    Pending(DeoxysPendingBlockInfo),
    NotPending(DeoxysBlockInfo),
}

impl DeoxysMaybePendingBlockInfo {
    pub fn as_nonpending(&self) -> Option<&DeoxysBlockInfo> {
        match self {
            DeoxysMaybePendingBlockInfo::Pending(_) => None,
            DeoxysMaybePendingBlockInfo::NotPending(v) => Some(v),
        }
    }
    pub fn as_pending(&self) -> Option<&DeoxysPendingBlockInfo> {
        match self {
            DeoxysMaybePendingBlockInfo::Pending(v) => Some(v),
            DeoxysMaybePendingBlockInfo::NotPending(_) => None,
        }
    }

    pub fn as_block_id(&self) -> BlockId {
        match self {
            DeoxysMaybePendingBlockInfo::Pending(_) => BlockId::Tag(BlockTag::Pending),
            DeoxysMaybePendingBlockInfo::NotPending(info) => BlockId::Number(info.header.block_number),
        }
    }

    pub fn block_n(&self) -> Option<u64> {
        self.as_nonpending().map(|v| v.header.block_number)
    }
    pub fn tx_hashes(&self) -> &[Felt] {
        match self {
            DeoxysMaybePendingBlockInfo::NotPending(block) => &block.tx_hashes,
            DeoxysMaybePendingBlockInfo::Pending(block) => &block.tx_hashes,
        }
    }

    pub fn protocol_version(&self) -> &StarknetVersion {
        match self {
            DeoxysMaybePendingBlockInfo::NotPending(block) => &block.header.protocol_version,
            DeoxysMaybePendingBlockInfo::Pending(block) => &block.header.protocol_version,
        }
    }
}

impl From<DeoxysPendingBlockInfo> for DeoxysMaybePendingBlockInfo {
    fn from(value: DeoxysPendingBlockInfo) -> Self {
        Self::Pending(value)
    }
}
impl From<DeoxysBlockInfo> for DeoxysMaybePendingBlockInfo {
    fn from(value: DeoxysBlockInfo) -> Self {
        Self::NotPending(value)
    }
}

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
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct DeoxysPendingBlockInfo {
    pub header: PendingHeader,
    pub tx_hashes: Vec<Felt>,
}

impl DeoxysPendingBlockInfo {
    pub fn new(header: PendingHeader, tx_hashes: Vec<Felt>) -> Self {
        Self { header, tx_hashes }
    }
}

// Light version of the block with block_hash
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct DeoxysBlockInfo {
    pub header: Header,
    pub block_hash: Felt,
    pub tx_hashes: Vec<Felt>,
}

impl DeoxysBlockInfo {
    pub fn new(header: Header, tx_hashes: Vec<Felt>, block_hash: Felt) -> Self {
        Self { header, block_hash, tx_hashes }
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct DeoxysBlockInner {
    /// The block transactions.
    pub transactions: Vec<Transaction>,
    /// The block transactions receipts.
    pub receipts: Vec<TransactionReceipt>,
}

impl DeoxysBlockInner {
    pub fn new(transactions: Vec<Transaction>, receipts: Vec<TransactionReceipt>) -> Self {
        Self { transactions, receipts }
    }
}

/// Starknet block definition.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct DeoxysMaybePendingBlock {
    pub info: DeoxysMaybePendingBlockInfo,
    pub inner: DeoxysBlockInner,
}

/// Starknet block definition.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct DeoxysBlock {
    pub info: DeoxysBlockInfo,
    pub inner: DeoxysBlockInner,
}

impl DeoxysBlock {
    /// Creates a new block.
    pub fn new(info: DeoxysBlockInfo, inner: DeoxysBlockInner) -> Self {
        Self { info, inner }
    }
}

/// Starknet block definition.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct DeoxysPendingBlock {
    pub info: DeoxysPendingBlockInfo,
    pub inner: DeoxysBlockInner,
}

impl DeoxysPendingBlock {
    /// Creates a new block.
    pub fn new(info: DeoxysPendingBlockInfo, inner: DeoxysBlockInner) -> Self {
        Self { info, inner }
    }
}
