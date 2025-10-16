//! Starknet block primitives.

use crate::header::GasPrices;
use commitments::{BlockCommitments, CommitmentComputationContext};
use header::BlockTimestamp;
use mp_chain_config::L1DataAvailabilityMode;
use mp_chain_config::StarknetVersion;
use mp_convert::FixedPoint;
use mp_receipt::{EventWithTransactionHash, TransactionReceipt};
use mp_state_update::StateDiff;
use starknet_types_core::felt::Felt;

pub mod commitments;
pub mod event_with_info;
pub mod header;
pub mod to_rpc;

pub use event_with_info::EventWithInfo;
pub use header::{Header, PreconfirmedHeader};
pub use mp_transactions::Transaction;
pub use primitive_types::{H160, U256};

// TODO: where should we put that?
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct TransactionWithReceipt {
    pub transaction: Transaction,
    pub receipt: TransactionReceipt,
}

impl TransactionWithReceipt {
    pub fn contract_address(&self) -> &Felt {
        match &self.transaction {
            Transaction::Invoke(tx) => tx.sender_address(),
            Transaction::L1Handler(tx) => &tx.contract_address,
            Transaction::Declare(tx) => tx.sender_address(),
            // Hash is in the receipt for DeployAccount.
            Transaction::Deploy(_) | Transaction::DeployAccount(_) => {
                self.receipt.contract_address().expect("Malformated receipt")
            }
        }
    }
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

impl BlockHeaderWithSignatures {
    pub fn new_unsigned(header: Header, block_hash: Felt) -> Self {
        Self { header, block_hash, consensus_signatures: vec![] }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[allow(clippy::large_enum_variant)]
pub enum MadaraMaybePreconfirmedBlockInfo {
    Preconfirmed(MadaraPreconfirmedBlockInfo),
    Confirmed(MadaraBlockInfo),
}

impl MadaraMaybePreconfirmedBlockInfo {
    pub fn as_closed(&self) -> Option<&MadaraBlockInfo> {
        match self {
            Self::Preconfirmed(_) => None,
            Self::Confirmed(v) => Some(v),
        }
    }

    pub fn into_closed(self) -> Option<MadaraBlockInfo> {
        match self {
            Self::Preconfirmed(_) => None,
            Self::Confirmed(v) => Some(v),
        }
    }

    pub fn as_pending(&self) -> Option<&MadaraPreconfirmedBlockInfo> {
        match self {
            Self::Preconfirmed(v) => Some(v),
            Self::Confirmed(_) => None,
        }
    }

    pub fn into_pending(self) -> Option<MadaraPreconfirmedBlockInfo> {
        match self {
            Self::Preconfirmed(v) => Some(v),
            Self::Confirmed(_) => None,
        }
    }

    pub fn block_number(&self) -> u64 {
        match self {
            Self::Preconfirmed(info) => info.header.block_number,
            Self::Confirmed(info) => info.header.block_number,
        }
    }

    pub fn block_hash(&self) -> Option<&Felt> {
        self.as_closed().map(|v| &v.block_hash)
    }

    pub fn tx_hashes(&self) -> &[Felt] {
        match self {
            Self::Confirmed(block) => &block.tx_hashes,
            Self::Preconfirmed(block) => &block.tx_hashes,
        }
    }

    pub fn protocol_version(&self) -> &StarknetVersion {
        match self {
            Self::Confirmed(block) => &block.header.protocol_version,
            Self::Preconfirmed(block) => &block.header.protocol_version,
        }
    }
    pub fn sequencer_address(&self) -> &Felt {
        match self {
            Self::Confirmed(block) => &block.header.sequencer_address,
            Self::Preconfirmed(block) => &block.header.sequencer_address,
        }
    }
    pub fn l1_da_mode(&self) -> &L1DataAvailabilityMode {
        match self {
            Self::Confirmed(block) => &block.header.l1_da_mode,
            Self::Preconfirmed(block) => &block.header.l1_da_mode,
        }
    }
    pub fn gas_prices(&self) -> &GasPrices {
        match self {
            Self::Confirmed(block) => &block.header.gas_prices,
            Self::Preconfirmed(block) => &block.header.gas_prices,
        }
    }
    pub fn block_timestamp(&self) -> &BlockTimestamp {
        match self {
            Self::Confirmed(block) => &block.header.block_timestamp,
            Self::Preconfirmed(block) => &block.header.block_timestamp,
        }
    }
}

impl From<MadaraPreconfirmedBlockInfo> for MadaraMaybePreconfirmedBlockInfo {
    fn from(value: MadaraPreconfirmedBlockInfo) -> Self {
        Self::Preconfirmed(value)
    }
}

impl From<MadaraBlockInfo> for MadaraMaybePreconfirmedBlockInfo {
    fn from(value: MadaraBlockInfo) -> Self {
        Self::Confirmed(value)
    }
}

// Light version of the block with block_hash
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MadaraPreconfirmedBlockInfo {
    pub header: PreconfirmedHeader,
    pub tx_hashes: Vec<Felt>,
}

impl MadaraPreconfirmedBlockInfo {
    pub fn new(header: PreconfirmedHeader, tx_hashes: Vec<Felt>) -> Self {
        Self { header, tx_hashes }
    }
}

// Light version of the block with block_hash
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MadaraBlockInfo {
    pub header: Header,
    pub block_hash: Felt,
    pub total_l2_gas_used: u128,
    pub tx_hashes: Vec<Felt>,
}

impl MadaraBlockInfo {
    pub fn new(header: Header, tx_hashes: Vec<Felt>, block_hash: Felt, total_l2_gas_used: u128) -> Self {
        Self { header, block_hash, tx_hashes, total_l2_gas_used }
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
                .map(|event| EventWithTransactionHash { transaction_hash: *r.transaction_hash(), event })
        })
    }
}

/// Starknet block definition.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MadaraMaybePendingBlock {
    pub info: MadaraMaybePreconfirmedBlockInfo,
    pub inner: MadaraBlockInner,
}

impl MadaraMaybePendingBlock {
    pub fn is_pending(&self) -> bool {
        matches!(self.info, MadaraMaybePreconfirmedBlockInfo::Preconfirmed(_))
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
    pub info: MadaraPreconfirmedBlockInfo,
    pub inner: MadaraBlockInner,
}

impl MadaraPendingBlock {
    /// Creates a new block.
    pub fn new(info: MadaraPreconfirmedBlockInfo, inner: MadaraBlockInner) -> Self {
        Self { info, inner }
    }

    pub fn new_empty(header: PreconfirmedHeader) -> Self {
        Self {
            info: MadaraPreconfirmedBlockInfo { header, tx_hashes: vec![] },
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
            MadaraMaybePreconfirmedBlockInfo::Preconfirmed(_) => Err(BlockIsPendingError(())),
            MadaraMaybePreconfirmedBlockInfo::Confirmed(info) => Ok(MadaraBlock { info, inner: value.inner }),
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
            MadaraMaybePreconfirmedBlockInfo::Confirmed(_) => Err(BlockIsNotPendingError(())),
            MadaraMaybePreconfirmedBlockInfo::Preconfirmed(info) => Ok(MadaraPendingBlock { info, inner: value.inner }),
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

#[derive(Clone, Debug, Default)]
pub struct FullBlockWithoutCommitments {
    pub header: PreconfirmedHeader,
    pub state_diff: StateDiff,
    pub transactions: Vec<TransactionWithReceipt>,
    pub events: Vec<EventWithTransactionHash>,
}

impl FullBlockWithoutCommitments {
    /// Uses the rayon thread pool.
    pub fn close_block(
        self,
        ctx: &CommitmentComputationContext,
        parent_block_hash: Felt,
        new_global_state_root: Felt,
        pre_v0_13_2_override: bool,
    ) -> FullBlock {
        println!("Comes to FullBlockWithoutCommitments to compute hash");
        let commitments = BlockCommitments::compute(ctx, &self.transactions, &self.state_diff, &self.events);
        let header = self.header.into_confirmed_header(parent_block_hash, commitments, new_global_state_root);
        FullBlock {
            block_hash: header.compute_hash(ctx.chain_id, pre_v0_13_2_override),
            header,
            state_diff: self.state_diff,
            transactions: self.transactions,
            events: self.events,
        }
    }
}

/// Gas quote for calculating gas prices.
/// It's represents an instantaneous quote of current L1 network fees.
#[derive(Clone, Default, Debug)]
pub struct L1GasQuote {
    pub l1_gas_price: u128,
    pub l1_data_gas_price: u128,
    pub strk_per_eth: FixedPoint,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_maybe_pending_block_info() {
        let tx_hashes_pending = vec![Felt::from(1), Felt::from(2)];
        let pending = MadaraPreconfirmedBlockInfo::new(PreconfirmedHeader::default(), tx_hashes_pending.clone());
        let pending_as_maybe_pending: MadaraMaybePreconfirmedBlockInfo = pending.clone().into();
        let tx_hashes_not_pending = vec![Felt::from(3), Felt::from(4)];
        let not_pending = MadaraBlockInfo::new(Header::default(), tx_hashes_not_pending.clone(), Felt::from(5), 0);
        let not_pending_as_maybe_pending: MadaraMaybePreconfirmedBlockInfo = not_pending.clone().into();

        assert_eq!(not_pending_as_maybe_pending.as_closed(), Some(&not_pending));
        assert!(pending_as_maybe_pending.as_closed().is_none());

        assert_eq!(pending_as_maybe_pending.as_pending(), Some(&pending));
        assert!(not_pending_as_maybe_pending.as_pending().is_none());

        assert_eq!(pending_as_maybe_pending.block_number(), 0);
        assert_eq!(not_pending_as_maybe_pending.block_number(), 0);

        assert_eq!(pending_as_maybe_pending.tx_hashes(), &tx_hashes_pending);
        assert_eq!(not_pending_as_maybe_pending.tx_hashes(), &tx_hashes_not_pending);

        assert_eq!(pending_as_maybe_pending.protocol_version(), &StarknetVersion::LATEST);
        assert_eq!(not_pending_as_maybe_pending.protocol_version(), &StarknetVersion::LATEST);

        let maybe_pending: MadaraMaybePendingBlock =
            MadaraPendingBlock::new_empty(PreconfirmedHeader::default()).into();
        assert!(MadaraBlock::try_from(maybe_pending.clone()).is_err());
        assert!(MadaraPendingBlock::try_from(maybe_pending.clone()).is_ok());
    }
}
