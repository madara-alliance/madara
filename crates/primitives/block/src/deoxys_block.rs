use starknet_types_core::felt::Felt;

use dp_chain_config::StarknetVersion;
use dp_receipt::TransactionReceipt;
use dp_transactions::Transaction;

use crate::header::{Header, PendingHeader};
use crate::{BlockId, BlockTag};

#[derive(Debug, thiserror::Error)]
#[error("Block is pending")]
pub struct BlockIsPendingError(());

impl TryFrom<DeoxysMaybePendingBlock> for DeoxysBlock {
    type Error = BlockIsPendingError;
    fn try_from(value: DeoxysMaybePendingBlock) -> Result<Self, Self::Error> {
        match value.info {
            DeoxysMaybePendingBlockInfo::Pending(_) => Err(BlockIsPendingError(())),
            DeoxysMaybePendingBlockInfo::NotPending(info) => Ok(DeoxysBlock { info, inner: value.inner }),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("Block is not pending")]
pub struct BlockIsNotPendingError(());

impl TryFrom<DeoxysMaybePendingBlock> for DeoxysPendingBlock {
    type Error = BlockIsNotPendingError;
    fn try_from(value: DeoxysMaybePendingBlock) -> Result<Self, Self::Error> {
        match value.info {
            DeoxysMaybePendingBlockInfo::NotPending(_) => Err(BlockIsNotPendingError(())),
            DeoxysMaybePendingBlockInfo::Pending(info) => Ok(DeoxysPendingBlock { info, inner: value.inner }),
        }
    }
}

impl From<DeoxysPendingBlock> for DeoxysMaybePendingBlock {
    fn from(value: DeoxysPendingBlock) -> Self {
        Self { info: value.info.into(), inner: value.inner }
    }
}
impl From<DeoxysBlock> for DeoxysMaybePendingBlock {
    fn from(value: DeoxysBlock) -> Self {
        Self { info: value.info.into(), inner: value.inner }
    }
}

// Light version of the block with block_hash
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DeoxysPendingBlockInfo {
    pub header: PendingHeader,
    pub tx_hashes: Vec<Felt>,
}

impl DeoxysPendingBlockInfo {
    pub fn new(header: PendingHeader, tx_hashes: Vec<Felt>) -> Self {
        Self { header, tx_hashes }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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

    pub fn block_hash(&self) -> Option<Felt> {
        self.as_nonpending().map(|v| v.block_hash)
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

// Light version of the block with block_hash
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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

/// Starknet block inner.
///
/// Contains the block transactions and receipts.
/// The transactions and receipts are in the same order.
/// The i-th transaction corresponds to the i-th receipt.
/// The length of the transactions and receipts must be the same.
///
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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

    pub fn new_empty(header: PendingHeader) -> Self {
        Self {
            info: DeoxysPendingBlockInfo { header, tx_hashes: vec![] },
            inner: DeoxysBlockInner { receipts: vec![], transactions: vec![] },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_maybe_pending_block_info() {
        let tx_hashes_pending = vec![Felt::from(1), Felt::from(2)];
        let pending = DeoxysPendingBlockInfo::new(PendingHeader::default(), tx_hashes_pending.clone());
        let pending_as_maybe_pending: DeoxysMaybePendingBlockInfo = pending.clone().into();
        let tx_hashes_not_pending = vec![Felt::from(3), Felt::from(4)];
        let not_pending = DeoxysBlockInfo::new(Header::default(), tx_hashes_not_pending.clone(), Felt::from(5));
        let not_pending_as_maybe_pending: DeoxysMaybePendingBlockInfo = not_pending.clone().into();

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

        assert_eq!(pending_as_maybe_pending.protocol_version(), &StarknetVersion::default());
        assert_eq!(not_pending_as_maybe_pending.protocol_version(), &StarknetVersion::default());

        let maybe_pending: DeoxysMaybePendingBlock = DeoxysPendingBlock::new_empty(PendingHeader::default()).into();
        assert!(DeoxysBlock::try_from(maybe_pending.clone()).is_err());
        assert!(DeoxysPendingBlock::try_from(maybe_pending.clone()).is_ok());
    }
}
