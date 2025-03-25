use super::{
    receipt::{ConfirmedReceipt, MsgToL2},
    transaction::Transaction,
};
use mp_block::{
    header::{L1DataAvailabilityMode, PendingHeader},
    FullBlock, PendingFullBlock, TransactionWithReceipt,
};
use mp_chain_config::{StarknetVersion, StarknetVersionError};
use mp_convert::hex_serde::U128AsHex;
use mp_receipt::EventWithTransactionHash;
use mp_state_update::StateDiff;
use serde::{Deserialize, Serialize};
use serde_with::serde_as;
use starknet_types_core::felt::Felt;
use std::mem;

#[derive(Debug, thiserror::Error)]
pub enum FromGatewayError {
    #[error("Transaction count is not equal to receipt count")]
    TransactionCountNotEqualToReceiptCount,
    #[error("Invalid starknet version: {0:#}")]
    StarknetVersion(#[from] StarknetVersionError),
    #[error("Missing protocol version field")]
    NoProtocolVersion,
    #[error("Unable to determine Starknet version for block {0:#x}")]
    FromMainnetStarknetVersion(Felt),
}

fn protocol_version_pending(starknet_version: Option<&str>) -> Result<StarknetVersion, FromGatewayError> {
    Ok(starknet_version.ok_or(FromGatewayError::NoProtocolVersion)?.parse()?)
}
fn protocol_version(
    starknet_version: Option<&str>,
    block_number: u64,
    block_hash: Felt,
) -> Result<StarknetVersion, FromGatewayError> {
    starknet_version.map(|version| Ok(version.parse()?)).unwrap_or_else(|| {
        StarknetVersion::try_from_mainnet_block_number(block_number)
            .ok_or(FromGatewayError::FromMainnetStarknetVersion(block_hash))
    })
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "deny_unknown_fields", serde(deny_unknown_fields))]
#[cfg_attr(test, derive(Eq))]
pub struct ProviderBlockHeader {
    pub block_number: u64,
    pub block_hash: Felt,
}

#[derive(Debug, Clone, PartialEq, Serialize)] // no Deserialize because it's untagged
#[serde(untagged)]
#[allow(clippy::large_enum_variant)]
pub enum ProviderBlockPendingMaybe {
    NonPending(ProviderBlock),
    Pending(ProviderBlockPending),
}

impl ProviderBlockPendingMaybe {
    pub fn non_pending(&self) -> Option<&ProviderBlock> {
        match self {
            ProviderBlockPendingMaybe::NonPending(non_pending) => Some(non_pending),
            ProviderBlockPendingMaybe::Pending(_) => None,
        }
    }

    pub fn non_pending_owned(self) -> Option<ProviderBlock> {
        match self {
            ProviderBlockPendingMaybe::NonPending(non_pending) => Some(non_pending),
            ProviderBlockPendingMaybe::Pending(_) => None,
        }
    }

    pub fn pending(&self) -> Option<&ProviderBlockPending> {
        match self {
            ProviderBlockPendingMaybe::NonPending(_) => None,
            ProviderBlockPendingMaybe::Pending(pending) => Some(pending),
        }
    }

    pub fn pending_owned(self) -> Option<ProviderBlockPending> {
        match self {
            ProviderBlockPendingMaybe::NonPending(_) => None,
            ProviderBlockPendingMaybe::Pending(pending) => Some(pending),
        }
    }

    pub fn parent_block_hash(&self) -> Felt {
        match self {
            ProviderBlockPendingMaybe::NonPending(non_pending) => non_pending.parent_block_hash,
            ProviderBlockPendingMaybe::Pending(pending) => pending.parent_block_hash,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "deny_unknown_fields", serde(deny_unknown_fields))]
#[cfg_attr(test, derive(Eq))]
pub struct ProviderBlock {
    pub block_hash: Felt,
    pub block_number: u64,
    pub parent_block_hash: Felt,
    pub timestamp: u64,
    #[serde(default)]
    pub sequencer_address: Option<Felt>,
    pub state_root: Felt,
    pub transaction_commitment: Felt,
    pub event_commitment: Felt,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receipt_commitment: Option<Felt>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_diff_commitment: Option<Felt>,
    #[serde(default)]
    pub state_diff_length: Option<u64>,
    pub status: BlockStatus,
    pub l1_da_mode: L1DataAvailabilityMode,
    pub l1_gas_price: ResourcePrice,
    pub l1_data_gas_price: ResourcePrice,
    pub transactions: Vec<Transaction>,
    pub transaction_receipts: Vec<ConfirmedReceipt>,
    #[serde(default)]
    pub starknet_version: Option<String>,
}

impl ProviderBlock {
    pub fn new(block: mp_block::MadaraBlock, status: BlockStatus) -> Self {
        let starknet_version = starknet_version(block.info.header.protocol_version);

        let transactions: Vec<_> = block
            .inner
            .transactions
            .into_iter()
            .zip(block.inner.receipts.iter().map(|receipt| (receipt.transaction_hash(), receipt.contract_address())))
            .map(|(transaction, (hash, contract_address))| {
                let transaction_with_hash = mp_transactions::TransactionWithHash { transaction, hash };
                Transaction::new(transaction_with_hash, contract_address)
            })
            .collect();

        let transaction_receipts = receipts(block.inner.receipts, &transactions);

        let sequencer_address = if block.info.header.sequencer_address == Felt::ZERO {
            None
        } else {
            Some(block.info.header.sequencer_address)
        };

        // TODO(compute_v0_13_2_hashes): once `compute_v0_13_2_hashes` becomes the default, we should show all post-v0.13.2 commitments
        // in the block including receipt and state_diff commitments.
        let (receipt_commitment, state_diff_commitment) =
            if block.info.header.protocol_version >= StarknetVersion::V0_13_2 {
                (block.info.header.receipt_commitment, block.info.header.state_diff_commitment)
            } else {
                (None, None)
            };

        Self {
            block_hash: block.info.block_hash,
            block_number: block.info.header.block_number,
            parent_block_hash: block.info.header.parent_block_hash,
            timestamp: block.info.header.block_timestamp.0,
            sequencer_address,
            state_root: block.info.header.global_state_root,
            transaction_commitment: block.info.header.transaction_commitment,
            event_commitment: block.info.header.event_commitment,
            receipt_commitment,
            state_diff_commitment,
            state_diff_length: block.info.header.state_diff_length,
            status,
            l1_da_mode: block.info.header.l1_da_mode,
            l1_gas_price: ResourcePrice {
                price_in_wei: block.info.header.l1_gas_price.eth_l1_gas_price,
                price_in_fri: block.info.header.l1_gas_price.strk_l1_gas_price,
            },
            l1_data_gas_price: ResourcePrice {
                price_in_wei: block.info.header.l1_gas_price.eth_l1_data_gas_price,
                price_in_fri: block.info.header.l1_gas_price.strk_l1_data_gas_price,
            },
            transactions,
            transaction_receipts,
            starknet_version,
        }
    }

    pub fn into_full_block(self, state_diff: StateDiff) -> Result<FullBlock, FromGatewayError> {
        if self.transactions.len() != self.transaction_receipts.len() {
            return Err(FromGatewayError::TransactionCountNotEqualToReceiptCount);
        }
        let header = self.header(&state_diff)?;
        let TransactionsReceiptsAndEvents { transactions, events } =
            convert_txs(self.transactions, self.transaction_receipts);
        Ok(FullBlock { block_hash: self.block_hash, header, transactions, events, state_diff })
    }

    pub fn header(&self, state_diff: &StateDiff) -> Result<mp_block::Header, FromGatewayError> {
        Ok(mp_block::Header {
            parent_block_hash: self.parent_block_hash,
            sequencer_address: self.sequencer_address.unwrap_or_default(),
            block_timestamp: mp_block::header::BlockTimestamp(self.timestamp),
            protocol_version: protocol_version(self.starknet_version.as_deref(), self.block_number, self.block_hash)?,
            l1_gas_price: mp_block::header::GasPrices {
                eth_l1_gas_price: self.l1_gas_price.price_in_wei,
                strk_l1_gas_price: self.l1_gas_price.price_in_fri,
                eth_l1_data_gas_price: self.l1_data_gas_price.price_in_wei,
                strk_l1_data_gas_price: self.l1_data_gas_price.price_in_fri,
            },
            l1_da_mode: self.l1_da_mode,
            block_number: self.block_number,
            global_state_root: self.state_root,
            transaction_count: self.transactions.len() as u64,
            transaction_commitment: self.transaction_commitment,
            event_count: self.transaction_receipts.iter().map(|r| r.events.len() as u64).sum(),
            event_commitment: self.event_commitment,
            state_diff_length: Some(state_diff.len() as u64),
            state_diff_commitment: self.state_diff_commitment,
            receipt_commitment: self.receipt_commitment,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "deny_unknown_fields", serde(deny_unknown_fields))]
#[cfg_attr(test, derive(Eq))]
pub struct ProviderBlockPending {
    pub parent_block_hash: Felt,
    pub status: BlockStatus,
    pub l1_da_mode: L1DataAvailabilityMode,
    pub l1_gas_price: ResourcePrice,
    pub l1_data_gas_price: ResourcePrice,
    pub transactions: Vec<Transaction>,
    pub timestamp: u64,
    #[serde(default)]
    pub sequencer_address: Felt,
    pub transaction_receipts: Vec<ConfirmedReceipt>,
    #[serde(default)]
    pub starknet_version: Option<String>,
}

impl ProviderBlockPending {
    pub fn new(block: mp_block::MadaraPendingBlock) -> Self {
        let starknet_version = starknet_version(block.info.header.protocol_version);

        let transactions: Vec<_> = block
            .inner
            .transactions
            .into_iter()
            .zip(block.inner.receipts.iter().map(|receipt| (receipt.transaction_hash(), receipt.contract_address())))
            .map(|(transaction, (hash, contract_address))| {
                let transaction_with_hash = mp_transactions::TransactionWithHash { transaction, hash };
                Transaction::new(transaction_with_hash, contract_address)
            })
            .collect();

        let transaction_receipts = receipts(block.inner.receipts, &transactions);

        Self {
            parent_block_hash: block.info.header.parent_block_hash,
            status: BlockStatus::Pending,
            l1_da_mode: block.info.header.l1_da_mode,
            l1_gas_price: ResourcePrice {
                price_in_wei: block.info.header.l1_gas_price.eth_l1_gas_price,
                price_in_fri: block.info.header.l1_gas_price.strk_l1_gas_price,
            },
            l1_data_gas_price: ResourcePrice {
                price_in_wei: block.info.header.l1_gas_price.eth_l1_data_gas_price,
                price_in_fri: block.info.header.l1_gas_price.strk_l1_data_gas_price,
            },
            transactions,
            timestamp: block.info.header.block_timestamp.0,
            sequencer_address: block.info.header.sequencer_address,
            transaction_receipts,
            starknet_version,
        }
    }

    pub fn header(&self) -> Result<PendingHeader, FromGatewayError> {
        Ok(PendingHeader {
            parent_block_hash: self.parent_block_hash,
            sequencer_address: self.sequencer_address,
            block_timestamp: mp_block::header::BlockTimestamp(self.timestamp),
            protocol_version: protocol_version_pending(self.starknet_version.as_deref())?,
            l1_gas_price: mp_block::header::GasPrices {
                eth_l1_gas_price: self.l1_gas_price.price_in_wei,
                strk_l1_gas_price: self.l1_gas_price.price_in_fri,
                eth_l1_data_gas_price: self.l1_data_gas_price.price_in_wei,
                strk_l1_data_gas_price: self.l1_data_gas_price.price_in_fri,
            },
            l1_da_mode: self.l1_da_mode,
        })
    }

    pub fn into_full_block(self, state_diff: StateDiff) -> Result<PendingFullBlock, FromGatewayError> {
        let header = self.header()?;
        let TransactionsReceiptsAndEvents { transactions, events } =
            convert_txs(self.transactions, self.transaction_receipts);
        Ok(PendingFullBlock { header, transactions, events, state_diff })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "deny_unknown_fields", serde(deny_unknown_fields))]
#[cfg_attr(test, derive(Eq))]
pub struct ProviderBlockSignature {
    pub block_hash: Felt,
    pub signature: Vec<Felt>,
}

#[serde_as]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "deny_unknown_fields", serde(deny_unknown_fields))]
#[cfg_attr(test, derive(Eq))]
pub struct ResourcePrice {
    #[serde_as(as = "U128AsHex")]
    pub price_in_wei: u128,
    #[serde_as(as = "U128AsHex")]
    pub price_in_fri: u128,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[cfg_attr(feature = "deny_unknown_fields", serde(deny_unknown_fields))]
pub enum BlockStatus {
    Pending,
    Aborted,
    Reverted,
    AcceptedOnL2,
    AcceptedOnL1,
}

fn starknet_version(version: StarknetVersion) -> Option<String> {
    match version {
        version if version < StarknetVersion::V0_9_1 => None,
        version => Some(version.to_string()),
    }
}

fn receipts(receipts: Vec<mp_receipt::TransactionReceipt>, transaction: &[Transaction]) -> Vec<ConfirmedReceipt> {
    receipts
        .into_iter()
        .zip(transaction.iter())
        .enumerate()
        .map(|(index, (receipt, tx))| {
            let l1_to_l2_consumed_message: Option<MsgToL2> = match tx {
                Transaction::L1Handler(l1_handler) => MsgToL2::try_from(l1_handler).ok(),
                _ => None,
            };
            ConfirmedReceipt::new(receipt, l1_to_l2_consumed_message, index as u64)
        })
        .collect()
}

struct TransactionsReceiptsAndEvents {
    transactions: Vec<TransactionWithReceipt>,
    events: Vec<EventWithTransactionHash>,
}

fn convert_txs(transactions: Vec<Transaction>, mut receipts: Vec<ConfirmedReceipt>) -> TransactionsReceiptsAndEvents {
    TransactionsReceiptsAndEvents {
        events: receipts
            .iter_mut()
            .flat_map(|receipt| {
                mem::take(&mut receipt.events)
                    .into_iter()
                    .map(|event| EventWithTransactionHash { transaction_hash: receipt.transaction_hash, event })
            })
            .collect(),
        transactions: transactions
            .into_iter()
            .zip(receipts)
            .map(|(transaction, receipt)| TransactionWithReceipt {
                receipt: receipt.into_mp(&transaction),
                transaction: transaction.into(),
            })
            .collect(),
    }
}
