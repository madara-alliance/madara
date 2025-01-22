//! Starknet block primitives.

use crate::header::GasPrices;
use header::{BlockTimestamp, L1DataAvailabilityMode, PendingHeader};
use mp_chain_config::StarknetVersion;
use mp_receipt::TransactionReceipt;
use mp_transactions::Transaction;
use starknet_types_core::felt::Felt;

pub mod header;
pub use header::Header;
pub use primitive_types::{H160, U256};
pub type BlockId = starknet_types_rpc::BlockId<Felt>;
pub type BlockTag = starknet_types_rpc::BlockTag;

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

    pub fn as_nonpending_owned(self) -> Option<MadaraBlockInfo> {
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

    pub fn block_timestamp(&self) -> BlockTimestamp {
        match self {
            MadaraMaybePendingBlockInfo::NotPending(block) => block.header.block_timestamp,
            MadaraMaybePendingBlockInfo::Pending(block) => block.header.block_timestamp,
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

impl From<MadaraBlockInfo> for starknet_types_rpc::BlockHeader<Felt> {
    fn from(info: MadaraBlockInfo) -> Self {
        let MadaraBlockInfo {
            header:
                Header {
                    parent_block_hash: parent_hash,
                    block_number,
                    global_state_root: new_root,
                    sequencer_address,
                    block_timestamp: timestamp,
                    protocol_version,
                    l1_gas_price,
                    l1_da_mode,
                    ..
                },
            block_hash,
            ..
        } = info;
        let GasPrices { eth_l1_gas_price, strk_l1_gas_price, eth_l1_data_gas_price, strk_l1_data_gas_price } =
            l1_gas_price;

        Self {
            block_hash,
            block_number,
            l1_da_mode: match l1_da_mode {
                L1DataAvailabilityMode::Blob => starknet_types_rpc::L1DaMode::Blob,
                L1DataAvailabilityMode::Calldata => starknet_types_rpc::L1DaMode::Calldata,
            },
            l1_data_gas_price: starknet_types_rpc::ResourcePrice {
                price_in_fri: Felt::from(strk_l1_data_gas_price),
                price_in_wei: Felt::from(eth_l1_data_gas_price),
            },
            l1_gas_price: starknet_types_rpc::ResourcePrice {
                price_in_fri: Felt::from(strk_l1_gas_price),
                price_in_wei: Felt::from(eth_l1_gas_price),
            },
            new_root,
            parent_hash,
            sequencer_address,
            starknet_version: if protocol_version < StarknetVersion::V0_9_1 {
                "".to_string()
            } else {
                protocol_version.to_string()
            },
            timestamp: timestamp.0,
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

impl MadaraMaybePendingBlock {
    pub fn is_pending(&self) -> bool {
        matches!(self.info, MadaraMaybePendingBlockInfo::Pending(_))
    }
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

/// Visited segments are the class segments that are visited during the execution of the block.
/// This info is an input of SNOS and used for proving.
#[derive(Clone, Default, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct VisitedSegments(pub Vec<VisitedSegmentEntry>);

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct VisitedSegmentEntry {
    pub class_hash: Felt,
    pub segments: Vec<usize>,
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
}
