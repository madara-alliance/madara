//! Starknet block primitives.

use crate::header::GasPrices;
use commitments::{BlockCommitments, CommitmentComputationContext};
use header::{BlockTimestamp, L1DataAvailabilityMode, PendingHeader};
use mp_chain_config::StarknetVersion;
use mp_receipt::{EventWithTransactionHash, TransactionReceipt};
use mp_state_update::StateDiff;
use mp_transactions::Transaction;
use starknet_types_core::felt::Felt;

pub mod commitments;
pub mod header;

pub use header::Header;
pub use primitive_types::{H160, U256};

pub type BlockId = mp_rpc::BlockId;
pub type BlockTag = mp_rpc::BlockTag;

// TODO: where should we put that?
#[derive(Debug, Clone)]
pub struct TransactionWithReceipt {
    pub transaction: Transaction,
    pub receipt: TransactionReceipt,
}

#[derive(Debug, Clone)]
pub struct ConsensusSignature {
    pub r: Felt,
    pub s: Felt,
}

#[derive(Debug, Clone)]
pub struct BlockHeaderWithSignatures {
    pub header: Header,
    pub block_hash: Felt,
    pub consensus_signatures: Vec<ConsensusSignature>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[allow(clippy::large_enum_variant)]
pub enum MadaraMaybePendingBlockInfo {
    Pending(MadaraPendingBlockInfo),
    NotPending(MadaraBlockInfo),
}

impl MadaraMaybePendingBlockInfo {
    pub fn as_closed(&self) -> Option<&MadaraBlockInfo> {
        match self {
            MadaraMaybePendingBlockInfo::Pending(_) => None,
            MadaraMaybePendingBlockInfo::NotPending(v) => Some(v),
        }
    }

    pub fn into_closed(self) -> Option<MadaraBlockInfo> {
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

    pub fn into_pending(self) -> Option<MadaraPendingBlockInfo> {
        match self {
            MadaraMaybePendingBlockInfo::Pending(v) => Some(v),
            MadaraMaybePendingBlockInfo::NotPending(_) => None,
        }
    }

    pub fn block_id(&self) -> BlockId {
        match self {
            MadaraMaybePendingBlockInfo::Pending(_) => BlockId::Tag(BlockTag::Pending),
            MadaraMaybePendingBlockInfo::NotPending(info) => BlockId::Number(info.header.block_number),
        }
    }

    pub fn block_n(&self) -> Option<u64> {
        self.as_closed().map(|v| v.header.block_number)
    }

    pub fn block_hash(&self) -> Option<Felt> {
        self.as_closed().map(|v| v.block_hash)
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

impl From<MadaraBlockInfo> for mp_rpc::BlockHeader {
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
                L1DataAvailabilityMode::Blob => mp_rpc::L1DaMode::Blob,
                L1DataAvailabilityMode::Calldata => mp_rpc::L1DaMode::Calldata,
            },
            l1_data_gas_price: mp_rpc::ResourcePrice {
                price_in_fri: Felt::from(strk_l1_data_gas_price),
                price_in_wei: Felt::from(eth_l1_data_gas_price),
            },
            l1_gas_price: mp_rpc::ResourcePrice {
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

    pub fn events(&self) -> impl Iterator<Item = EventWithTransactionHash> + '_ {
        self.receipts.iter().flat_map(|r| {
            r.events()
                .iter()
                .cloned()
                .map(|event| EventWithTransactionHash { transaction_hash: r.transaction_hash(), event })
        })
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

    pub fn into_pending(self) -> Option<MadaraPendingBlock> {
        Some(MadaraPendingBlock { info: self.info.into_pending()?, inner: self.inner })
    }
    pub fn into_closed(self) -> Option<MadaraBlock> {
        Some(MadaraBlock { info: self.info.into_closed()?, inner: self.inner })
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

#[derive(Clone, Debug)]
pub struct FullBlock {
    pub block_hash: Felt,
    pub header: Header,
    pub state_diff: StateDiff,
    pub transactions: Vec<TransactionWithReceipt>,
    pub events: Vec<EventWithTransactionHash>,
}

/// A pending block is a block that has not yet been closed.
#[derive(Clone, Debug)]
pub struct PendingFullBlock {
    pub header: PendingHeader,
    pub state_diff: StateDiff,
    pub transactions: Vec<TransactionWithReceipt>,
    pub events: Vec<EventWithTransactionHash>,
}

impl PendingFullBlock {
    /// Uses the rayon thread pool.
    pub fn close_block(
        self,
        ctx: &CommitmentComputationContext,
        block_number: u64,
        new_global_state_root: Felt,
        pre_v0_13_2_override: bool,
    ) -> FullBlock {
        let commitments = BlockCommitments::compute(ctx, &self.transactions, &self.state_diff, &self.events);
        let header = self.header.to_closed_header(commitments, new_global_state_root, block_number);
        FullBlock {
            block_hash: header.compute_hash(ctx.chain_id, pre_v0_13_2_override),
            header,
            state_diff: self.state_diff,
            transactions: self.transactions,
            events: self.events,
        }
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

        assert_eq!(not_pending_as_maybe_pending.as_closed(), Some(&not_pending));
        assert!(pending_as_maybe_pending.as_closed().is_none());

        assert_eq!(pending_as_maybe_pending.as_pending(), Some(&pending));
        assert!(not_pending_as_maybe_pending.as_pending().is_none());

        assert_eq!(pending_as_maybe_pending.block_id(), BlockId::Tag(BlockTag::Pending));
        assert_eq!(not_pending_as_maybe_pending.block_id(), BlockId::Number(0));

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
