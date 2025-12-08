use super::{receipt::ConfirmedReceipt, transaction::Transaction};
use crate::state_update::StateDiff;
use itertools::Itertools;
use mp_block::header::PreconfirmedHeader;
use mp_block::Header;
use mp_block::{FullBlock, TransactionWithReceipt};
use mp_chain_config::L1DataAvailabilityMode;
use mp_chain_config::{StarknetVersion, StarknetVersionError};
use mp_convert::hex_serde::U128AsHex;
use mp_receipt::EventWithTransactionHash;
use mp_state_update::TransactionStateUpdate;
use mp_transactions::validated::{TransactionWithHashAndContractAddress, ValidatedTransaction};
use mp_transactions::TransactionWithHash;
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "deny_unknown_fields", serde(deny_unknown_fields))]
#[cfg_attr(test, derive(Eq))]
pub struct ProviderBlock {
    pub block_hash: Felt,
    pub block_number: u64,
    pub parent_block_hash: Felt,
    pub timestamp: u64,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_diff_length: Option<u64>,
    pub status: BlockStatus,
    pub l1_da_mode: L1DataAvailabilityMode,
    pub l1_gas_price: ResourcePrice,
    pub l1_data_gas_price: ResourcePrice,
    pub l2_gas_price: ResourcePrice,
    pub transactions: Vec<Transaction>,
    pub transaction_receipts: Vec<ConfirmedReceipt>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub starknet_version: Option<String>,
}

impl ProviderBlock {
    pub fn new(block_hash: Felt, header: Header, content: Vec<TransactionWithReceipt>, status: BlockStatus) -> Self {
        let starknet_version = starknet_version(header.protocol_version);

        let (transactions, transaction_receipts): (Vec<_>, Vec<_>) = content
            .into_iter()
            .enumerate()
            .map(|(index, tx)| {
                let l1_to_l2_consumed_message = match &tx.transaction {
                    mp_block::Transaction::L1Handler(l1_handler) => {
                        mp_receipt::MsgToL2::try_from(&l1_handler.clone()).ok()
                    }
                    _ => None,
                };
                (
                    Transaction::new(
                        TransactionWithHash { transaction: tx.transaction, hash: *tx.receipt.transaction_hash() },
                        tx.receipt.contract_address().copied(),
                    ),
                    ConfirmedReceipt::new(tx.receipt, l1_to_l2_consumed_message, index as u64),
                )
            })
            .unzip();

        let sequencer_address =
            if header.sequencer_address == Felt::ZERO { None } else { Some(header.sequencer_address) };

        // TODO(compute_v0_13_2_hashes): once `compute_v0_13_2_hashes` becomes the default, we should show all post-v0.13.2 commitments
        // in the block including receipt and state_diff commitments.
        let (receipt_commitment, state_diff_commitment) = if header.protocol_version >= StarknetVersion::V0_13_2 {
            (header.receipt_commitment, header.state_diff_commitment)
        } else {
            (None, None)
        };

        Self {
            block_hash,
            block_number: header.block_number,
            parent_block_hash: header.parent_block_hash,
            timestamp: header.block_timestamp.0,
            sequencer_address,
            state_root: header.global_state_root,
            transaction_commitment: header.transaction_commitment,
            event_commitment: header.event_commitment,
            receipt_commitment,
            state_diff_commitment,
            state_diff_length: header.state_diff_length,
            status,
            l1_da_mode: header.l1_da_mode,
            l1_gas_price: ResourcePrice {
                price_in_wei: header.gas_prices.eth_l1_gas_price,
                price_in_fri: header.gas_prices.strk_l1_gas_price,
            },
            l1_data_gas_price: ResourcePrice {
                price_in_wei: header.gas_prices.eth_l1_data_gas_price,
                price_in_fri: header.gas_prices.strk_l1_data_gas_price,
            },
            l2_gas_price: ResourcePrice {
                price_in_wei: header.gas_prices.eth_l2_gas_price,
                price_in_fri: header.gas_prices.strk_l2_gas_price,
            },
            transactions,
            transaction_receipts,
            starknet_version,
        }
    }

    pub fn into_full_block(self, state_diff: mp_state_update::StateDiff) -> Result<FullBlock, FromGatewayError> {
        if self.transactions.len() != self.transaction_receipts.len() {
            return Err(FromGatewayError::TransactionCountNotEqualToReceiptCount);
        }
        let header = self.header(&state_diff)?;
        let TransactionsReceiptsAndEvents { transactions, events } =
            convert_txs(self.transactions, self.transaction_receipts);
        Ok(FullBlock { block_hash: self.block_hash, header, transactions, events, state_diff })
    }

    pub fn header(&self, state_diff: &mp_state_update::StateDiff) -> Result<mp_block::Header, FromGatewayError> {
        Ok(mp_block::Header {
            parent_block_hash: self.parent_block_hash,
            sequencer_address: self.sequencer_address.unwrap_or_default(),
            block_timestamp: mp_block::header::BlockTimestamp(self.timestamp),
            protocol_version: protocol_version(self.starknet_version.as_deref(), self.block_number, self.block_hash)?,
            gas_prices: mp_block::header::GasPrices {
                eth_l1_gas_price: self.l1_gas_price.price_in_wei,
                strk_l1_gas_price: self.l1_gas_price.price_in_fri,
                eth_l1_data_gas_price: self.l1_data_gas_price.price_in_wei,
                strk_l1_data_gas_price: self.l1_data_gas_price.price_in_fri,
                eth_l2_gas_price: self.l2_gas_price.price_in_wei,
                strk_l2_gas_price: self.l2_gas_price.price_in_fri,
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
pub struct ProviderBlockPreConfirmed {
    pub status: BlockStatus,
    pub starknet_version: String,
    pub l1_da_mode: L1DataAvailabilityMode,
    pub l1_gas_price: ResourcePrice,
    pub l1_data_gas_price: ResourcePrice,
    pub l2_gas_price: ResourcePrice,
    pub timestamp: u64,
    pub sequencer_address: Felt,
    pub transactions: Vec<Transaction>,
    pub transaction_receipts: Vec<Option<ConfirmedReceipt>>,
    pub transaction_state_diffs: Vec<Option<StateDiff>>,
}

impl ProviderBlockPreConfirmed {
    pub fn new<'a>(
        header: &PreconfirmedHeader,
        executed_txs: impl IntoIterator<Item = (&'a TransactionWithReceipt, &'a TransactionStateUpdate)>,
        candidates: impl IntoIterator<Item = &'a ValidatedTransaction>,
        status: BlockStatus,
    ) -> Self {
        let (transactions, transaction_receipts, transaction_state_diffs): (Vec<_>, Vec<_>, Vec<_>) = executed_txs
            .into_iter()
            .enumerate()
            .map(|(index, (tx, state_diff))| {
                let l1_to_l2_consumed_message = match &tx.transaction {
                    mp_block::Transaction::L1Handler(l1_handler) => {
                        mp_receipt::MsgToL2::try_from(&l1_handler.clone()).ok()
                    }
                    _ => None,
                };

                (
                    Transaction::new(
                        TransactionWithHash {
                            transaction: tx.transaction.clone(),
                            hash: *tx.receipt.transaction_hash(),
                        },
                        tx.receipt.contract_address().copied(),
                    ),
                    Some(ConfirmedReceipt::new(tx.receipt.clone(), l1_to_l2_consumed_message, index as u64)),
                    Some(state_diff.to_state_diff().into()),
                )
            })
            .chain(candidates.into_iter().map(|tx| {
                (
                    Transaction::new(
                        TransactionWithHash { transaction: tx.transaction.clone(), hash: tx.hash },
                        Some(tx.contract_address),
                    ),
                    None,
                    None,
                )
            }))
            .multiunzip();

        Self {
            timestamp: header.block_timestamp.0,
            sequencer_address: header.sequencer_address,
            status,
            l1_da_mode: header.l1_da_mode,
            l1_gas_price: ResourcePrice {
                price_in_wei: header.gas_prices.eth_l1_gas_price,
                price_in_fri: header.gas_prices.strk_l1_gas_price,
            },
            l1_data_gas_price: ResourcePrice {
                price_in_wei: header.gas_prices.eth_l1_data_gas_price,
                price_in_fri: header.gas_prices.strk_l1_data_gas_price,
            },
            l2_gas_price: ResourcePrice {
                price_in_wei: header.gas_prices.eth_l2_gas_price,
                price_in_fri: header.gas_prices.strk_l2_gas_price,
            },
            transactions,
            transaction_receipts,
            transaction_state_diffs,
            starknet_version: header.protocol_version.to_string(),
        }
    }

    pub fn header(&self, block_number: u64) -> Result<PreconfirmedHeader, FromGatewayError> {
        Ok(PreconfirmedHeader {
            sequencer_address: self.sequencer_address,
            block_timestamp: mp_block::header::BlockTimestamp(self.timestamp),
            protocol_version: self.starknet_version.parse()?,
            gas_prices: mp_block::header::GasPrices {
                eth_l1_gas_price: self.l1_gas_price.price_in_wei,
                strk_l1_gas_price: self.l1_gas_price.price_in_fri,
                eth_l1_data_gas_price: self.l1_data_gas_price.price_in_wei,
                strk_l1_data_gas_price: self.l1_data_gas_price.price_in_fri,
                eth_l2_gas_price: self.l2_gas_price.price_in_wei,
                strk_l2_gas_price: self.l2_gas_price.price_in_fri,
            },
            l1_da_mode: self.l1_da_mode,
            block_number,
        })
    }

    pub fn num_executed_transactions(&self) -> usize {
        self.transaction_receipts
            .iter()
            .zip(&self.transaction_state_diffs)
            .take_while(|(receipt, state_diff)| receipt.is_some() && state_diff.is_some())
            .count()
    }

    pub fn into_transactions(
        self,
        skip_first_n: usize,
    ) -> (Vec<(TransactionWithReceipt, TransactionStateUpdate)>, Vec<TransactionWithHashAndContractAddress>) {
        let mut transactions = self
            .transactions
            .into_iter()
            .zip(self.transaction_receipts)
            .zip(self.transaction_state_diffs)
            .skip(skip_first_n)
            .peekable();

        let executed = transactions
            .peeking_take_while(|((_transaction, receipt), state_diff)| receipt.is_some() && state_diff.is_some())
            .map(|((transaction, receipt), state_diff)| {
                (
                    TransactionWithReceipt {
                        receipt: receipt.expect("Should not be empty").into_mp(&transaction),
                        transaction: transaction.into(),
                    },
                    mp_state_update::StateDiff::from(state_diff.expect("Should not be empty")).into(),
                )
            })
            .collect();

        let candidates = transactions
            .map(|((tx, _), _)| TransactionWithHashAndContractAddress {
                contract_address: *tx.contract_address(),
                transaction: TransactionWithHash { hash: *tx.transaction_hash(), transaction: tx.into() },
            })
            .collect();

        (executed, candidates)
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
    #[serde(rename = "PENDING")]
    PreConfirmed,
    AcceptedOnL2,
    AcceptedOnL1,
}

fn starknet_version(version: StarknetVersion) -> Option<String> {
    match version {
        version if version < StarknetVersion::V0_9_1 => None,
        version => Some(version.to_string()),
    }
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
