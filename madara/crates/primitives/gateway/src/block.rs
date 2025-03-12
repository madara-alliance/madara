use super::{receipt::ConfirmedReceipt, transaction::Transaction};
use crate::transaction::L1HandlerTransaction;
use anyhow::Context;
use mp_block::header::{BlockTimestamp, L1DataAvailabilityMode};
use mp_chain_config::StarknetVersion;
use mp_convert::hex_serde::U128AsHex;
use serde::{Deserialize, Serialize};
use serde_with::serde_as;
use starknet_types_core::felt::Felt;

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
// #[serde(deny_unknown_fields)] // TODO(v0.13.4): Re-add this attribute when v0.13.4 is supported.
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

    pub fn header(&self) -> anyhow::Result<mc_block_import::UnverifiedHeader> {
        Ok(mc_block_import::UnverifiedHeader {
            parent_block_hash: Some(self.parent_block_hash),
            sequencer_address: self.sequencer_address.unwrap_or_default(),
            block_timestamp: BlockTimestamp(self.timestamp),
            protocol_version: self
                .starknet_version
                .as_deref()
                .map(|version| version.parse().context("Invalid Starknet version"))
                .unwrap_or_else(|| {
                    StarknetVersion::try_from_mainnet_block_number(self.block_number)
                        .context(format!("Unable to determine Starknet version for block {:#x}", self.block_hash))
                })?,
            l1_gas_price: mp_block::header::GasPrices {
                eth_l1_gas_price: self.l1_gas_price.price_in_wei,
                strk_l1_gas_price: self.l1_gas_price.price_in_fri,
                eth_l1_data_gas_price: self.l1_data_gas_price.price_in_wei,
                strk_l1_data_gas_price: self.l1_data_gas_price.price_in_fri,
            },
            l1_da_mode: self.l1_da_mode,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
// #[serde(deny_unknown_fields)] // TODO(v0.13.4): Re-add this attribute when v0.13.4 is supported.
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

    pub fn header(&self) -> anyhow::Result<mc_block_import::UnverifiedHeader> {
        Ok(mc_block_import::UnverifiedHeader {
            parent_block_hash: Some(self.parent_block_hash),
            sequencer_address: self.sequencer_address,
            block_timestamp: BlockTimestamp(self.timestamp),
            protocol_version: self
                .starknet_version
                .as_deref()
                .context(format!(
                    "Unable to determine Starknet version for pending block with parent hash {:#x}",
                    self.parent_block_hash
                ))?
                .parse()
                .context("Invalid Starknet version")?,
            l1_gas_price: mp_block::header::GasPrices {
                eth_l1_gas_price: self.l1_gas_price.price_in_wei,
                strk_l1_gas_price: self.l1_gas_price.price_in_fri,
                eth_l1_data_gas_price: self.l1_data_gas_price.price_in_wei,
                strk_l1_data_gas_price: self.l1_data_gas_price.price_in_fri,
            },
            l1_da_mode: self.l1_da_mode,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
// #[serde(deny_unknown_fields)] // TODO(v0.13.4): Re-add this attribute when v0.13.4 is supported.
#[cfg_attr(test, derive(Eq))]
pub struct ProviderBlockSignature {
    pub block_hash: Felt,
    pub signature: Vec<Felt>,
}

#[serde_as]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
// #[serde(deny_unknown_fields)] // TODO(v0.13.4): Re-add this attribute when v0.13.4 is supported.
#[cfg_attr(test, derive(Eq))]
pub struct ResourcePrice {
    #[serde_as(as = "U128AsHex")]
    pub price_in_wei: u128,
    #[serde_as(as = "U128AsHex")]
    pub price_in_fri: u128,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
// #[serde(deny_unknown_fields)] // TODO(v0.13.4): Re-add this attribute when v0.13.4 is supported.
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

fn l1_handler_to_msg_to_l2(l1_handler: &L1HandlerTransaction) -> Option<mp_receipt::MsgToL2> {
    let mp_l1_handler: mp_transactions::L1HandlerTransaction = l1_handler.clone().into();

    mp_receipt::MsgToL2::try_from(&mp_l1_handler).ok()
}

fn receipts(receipts: Vec<mp_receipt::TransactionReceipt>, transaction: &[Transaction]) -> Vec<ConfirmedReceipt> {
    receipts
        .into_iter()
        .zip(transaction.iter())
        .enumerate()
        .map(|(index, (receipt, tx))| {
            let l1_to_l2_consumed_message = match tx {
                Transaction::L1Handler(l1_handler) => l1_handler_to_msg_to_l2(l1_handler),
                _ => None,
            };
            ConfirmedReceipt::new(receipt, l1_to_l2_consumed_message, index as u64)
        })
        .collect()
}
