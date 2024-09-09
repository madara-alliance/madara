//! Starknet block primitives.

pub mod header;

pub use header::Header;
use header::PendingHeader;
use mp_chain_config::StarknetVersion;
use mp_receipt::TransactionReceipt;
use mp_transactions::Transaction;
pub use primitive_types::{H160, U256};
use starknet_types_core::felt::Felt;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[allow(clippy::large_enum_variant)]
pub enum MadaraMaybePendingBlockInfo {
    Pending(MadaraPendingBlockInfo),
    NotPending(MadaraBlockInfo),
}

impl MadaraMaybePendingBlockInfo {
    pub fn as_nonpending(&self) -> Option<&MadaraBlockInfo> {
        match self {
            MadaraMaybePendingBlockInfo::Pending(_) => None,
            MadaraMaybePendingBlockInfo::NotPending(v) => Some(v),
        }
    }
    pub fn as_pending(&self) -> Option<&MadaraPendingBlockInfo> {
        match self {
            MadaraMaybePendingBlockInfo::Pending(v) => Some(v),
            MadaraMaybePendingBlockInfo::NotPending(_) => None,
        }
    }

    pub fn as_block_id(&self) -> BlockId {
        match self {
            MadaraMaybePendingBlockInfo::Pending(_) => BlockId::Tag(BlockTag::Pending),
            MadaraMaybePendingBlockInfo::NotPending(info) => BlockId::Number(info.header.block_number),
        }
    }

    pub fn block_n(&self) -> Option<u64> {
        self.as_nonpending().map(|v| v.header.block_number)
    }

    pub fn block_hash(&self) -> Option<Felt> {
        self.as_nonpending().map(|v| v.block_hash)
    }

    pub fn tx_hashes(&self) -> &[Felt] {
        match self {
            MadaraMaybePendingBlockInfo::NotPending(block) => &block.tx_hashes,
            MadaraMaybePendingBlockInfo::Pending(block) => &block.tx_hashes,
        }
    }

    pub fn protocol_version(&self) -> &StarknetVersion {
        match self {
            MadaraMaybePendingBlockInfo::NotPending(block) => &block.header.protocol_version,
            MadaraMaybePendingBlockInfo::Pending(block) => &block.header.protocol_version,
        }
    }
}

impl From<MadaraPendingBlockInfo> for MadaraMaybePendingBlockInfo {
    fn from(value: MadaraPendingBlockInfo) -> Self {
        Self::Pending(value)
    }
}
impl From<MadaraBlockInfo> for MadaraMaybePendingBlockInfo {
    fn from(value: MadaraBlockInfo) -> Self {
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
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MadaraPendingBlockInfo {
    pub header: PendingHeader,
    pub tx_hashes: Vec<Felt>,
}

impl MadaraPendingBlockInfo {
    pub fn new(header: PendingHeader, tx_hashes: Vec<Felt>) -> Self {
        Self { header, tx_hashes }
    }
}

// Light version of the block with block_hash
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MadaraBlockInfo {
    pub header: Header,
    pub block_hash: Felt,
    pub tx_hashes: Vec<Felt>,
}

impl MadaraBlockInfo {
    pub fn new(header: Header, tx_hashes: Vec<Felt>, block_hash: Felt) -> Self {
        Self { header, block_hash, tx_hashes }
    }
}

/// Starknet block inner.
///
/// Contains the block transactions and receipts.
/// The transactions and receipts are in the same order.
/// The i-th transaction corresponds to the i-th receipt.
/// The length of the transactions and receipts must be the same.
///
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MadaraBlockInner {
    /// The block transactions.
    pub transactions: Vec<Transaction>,
    /// The block transactions receipts.
    pub receipts: Vec<TransactionReceipt>,
}

impl MadaraBlockInner {
    pub fn new(transactions: Vec<Transaction>, receipts: Vec<TransactionReceipt>) -> Self {
        Self { transactions, receipts }
    }
}

/// Starknet block definition.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MadaraMaybePendingBlock {
    pub info: MadaraMaybePendingBlockInfo,
    pub inner: MadaraBlockInner,
}

/// Starknet block definition.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct MadaraBlock {
    pub info: MadaraBlockInfo,
    pub inner: MadaraBlockInner,
}

impl MadaraBlock {
    /// Creates a new block.
    pub fn new(info: MadaraBlockInfo, inner: MadaraBlockInner) -> Self {
        Self { info, inner }
    }

    pub fn version(&self) -> StarknetVersion {
        self.info.header.protocol_version
    }
}

/// Starknet block definition.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct MadaraPendingBlock {
    pub info: MadaraPendingBlockInfo,
    pub inner: MadaraBlockInner,
}

impl MadaraPendingBlock {
    /// Creates a new block.
    pub fn new(info: MadaraPendingBlockInfo, inner: MadaraBlockInner) -> Self {
        Self { info, inner }
    }

    pub fn new_empty(header: PendingHeader) -> Self {
        Self {
            info: MadaraPendingBlockInfo { header, tx_hashes: vec![] },
            inner: MadaraBlockInner { receipts: vec![], transactions: vec![] },
        }
    }

    pub fn version(&self) -> StarknetVersion {
        self.info.header.protocol_version
    }
}

#[derive(Debug, thiserror::Error)]
#[error("Block is pending")]
pub struct BlockIsPendingError(());

impl TryFrom<MadaraMaybePendingBlock> for MadaraBlock {
    type Error = BlockIsPendingError;
    fn try_from(value: MadaraMaybePendingBlock) -> Result<Self, Self::Error> {
        match value.info {
            MadaraMaybePendingBlockInfo::Pending(_) => Err(BlockIsPendingError(())),
            MadaraMaybePendingBlockInfo::NotPending(info) => Ok(MadaraBlock { info, inner: value.inner }),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("Block is not pending")]
pub struct BlockIsNotPendingError(());

impl TryFrom<MadaraMaybePendingBlock> for MadaraPendingBlock {
    type Error = BlockIsNotPendingError;
    fn try_from(value: MadaraMaybePendingBlock) -> Result<Self, Self::Error> {
        match value.info {
            MadaraMaybePendingBlockInfo::NotPending(_) => Err(BlockIsNotPendingError(())),
            MadaraMaybePendingBlockInfo::Pending(info) => Ok(MadaraPendingBlock { info, inner: value.inner }),
        }
    }
}

impl From<MadaraPendingBlock> for MadaraMaybePendingBlock {
    fn from(value: MadaraPendingBlock) -> Self {
        Self { info: value.info.into(), inner: value.inner }
    }
}
impl From<MadaraBlock> for MadaraMaybePendingBlock {
    fn from(value: MadaraBlock) -> Self {
        Self { info: value.info.into(), inner: value.inner }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_maybe_pending_block_info() {
        let tx_hashes_pending = vec![Felt::from(1), Felt::from(2)];
        let pending = MadaraPendingBlockInfo::new(PendingHeader::default(), tx_hashes_pending.clone());
        let pending_as_maybe_pending: MadaraMaybePendingBlockInfo = pending.clone().into();
        let tx_hashes_not_pending = vec![Felt::from(3), Felt::from(4)];
        let not_pending = MadaraBlockInfo::new(Header::default(), tx_hashes_not_pending.clone(), Felt::from(5));
        let not_pending_as_maybe_pending: MadaraMaybePendingBlockInfo = not_pending.clone().into();

        assert_eq!(not_pending_as_maybe_pending.as_nonpending(), Some(&not_pending));
        assert!(pending_as_maybe_pending.as_nonpending().is_none());

        assert_eq!(pending_as_maybe_pending.as_pending(), Some(&pending));
        assert!(not_pending_as_maybe_pending.as_pending().is_none());

        assert_eq!(pending_as_maybe_pending.as_block_id(), BlockId::Tag(BlockTag::Pending));
        assert_eq!(not_pending_as_maybe_pending.as_block_id(), BlockId::Number(0));

        assert_eq!(pending_as_maybe_pending.block_n(), None);
        assert_eq!(not_pending_as_maybe_pending.block_n(), Some(0));

        assert_eq!(pending_as_maybe_pending.tx_hashes(), &tx_hashes_pending);
        assert_eq!(not_pending_as_maybe_pending.tx_hashes(), &tx_hashes_not_pending);

        assert_eq!(pending_as_maybe_pending.protocol_version(), &StarknetVersion::LATEST);
        assert_eq!(not_pending_as_maybe_pending.protocol_version(), &StarknetVersion::LATEST);

        let maybe_pending: MadaraMaybePendingBlock = MadaraPendingBlock::new_empty(PendingHeader::default()).into();
        assert!(MadaraBlock::try_from(maybe_pending.clone()).is_err());
        assert!(MadaraPendingBlock::try_from(maybe_pending.clone()).is_ok());
    }

    #[test]
    fn test_block_tag() {
        let tag = BlockTag::Latest;
        let tag_converted: starknet_core::types::BlockTag = tag.into();
        assert_eq!(tag_converted, starknet_core::types::BlockTag::Latest);
        let tag_back: BlockTag = tag_converted.into();
        assert_eq!(tag_back, tag);

        let tag = BlockTag::Pending;
        let tag_converted: starknet_core::types::BlockTag = tag.into();
        assert_eq!(tag_converted, starknet_core::types::BlockTag::Pending);
        let tag_back: BlockTag = tag_converted.into();
        assert_eq!(tag_back, tag);
    }

    #[test]
    fn test_block_id() {
        let hash = Felt::from(1);
        let hash_converted: starknet_core::types::BlockId = BlockId::Hash(hash).into();
        assert_eq!(hash_converted, starknet_core::types::BlockId::Hash(hash));
        let hash_back: BlockId = hash_converted.into();
        assert_eq!(hash_back, BlockId::Hash(hash));

        let number = 1;
        let number_converted: starknet_core::types::BlockId = BlockId::Number(number).into();
        assert_eq!(number_converted, starknet_core::types::BlockId::Number(number));
        let number_back: BlockId = number_converted.into();
        assert_eq!(number_back, BlockId::Number(number));

        let tag = BlockTag::Latest;
        let tag_converted: starknet_core::types::BlockId = BlockId::Tag(tag).into();
        assert_eq!(tag_converted, starknet_core::types::BlockId::Tag(starknet_core::types::BlockTag::Latest));
        let tag_back: BlockId = tag_converted.into();
        assert_eq!(tag_back, BlockId::Tag(BlockTag::Latest));
    }
}
