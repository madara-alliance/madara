use core::num::NonZeroU128;

use blockifier::versioned_constants::VersionedConstants;
use dp_transactions::MAIN_CHAIN_ID;
use starknet_types_core::felt::Felt;
use starknet_types_core::hash::Pedersen;
use starknet_types_core::hash::StarkHash as StarkHashTrait;

use crate::starknet_version::StarknetVersion;

/// Block status.
///
/// The status of the block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BlockStatus {
    Pending,
    #[default]
    AcceptedOnL2,
    AcceptedOnL1,
    Rejected,
}

impl From<BlockStatus> for starknet_core::types::BlockStatus {
    fn from(status: BlockStatus) -> Self {
        match status {
            BlockStatus::Pending => starknet_core::types::BlockStatus::Pending,
            BlockStatus::AcceptedOnL2 => starknet_core::types::BlockStatus::AcceptedOnL2,
            BlockStatus::AcceptedOnL1 => starknet_core::types::BlockStatus::AcceptedOnL1,
            BlockStatus::Rejected => starknet_core::types::BlockStatus::Rejected,
        }
    }
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct PendingHeader {
    /// The hash of this block’s parent.
    pub parent_block_hash: Felt,
    /// The Starknet address of the sequencer who created this block.
    pub sequencer_address: Felt,
    /// The time the sequencer created this block before executing transactions
    pub block_timestamp: u64,
    /// The version of the Starknet protocol used when creating this block
    pub protocol_version: StarknetVersion,
    /// Gas prices for this block
    pub l1_gas_price: GasPrices,
    /// The mode of data availability for this block
    pub l1_da_mode: L1DataAvailabilityMode,
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
/// Starknet header definition.
pub struct Header {
    /// The hash of this block’s parent.
    pub parent_block_hash: Felt,
    /// The number (height) of this block.
    pub block_number: u64,
    /// The state commitment after this block.
    pub global_state_root: Felt,
    /// The Starknet address of the sequencer who created this block.
    pub sequencer_address: Felt,
    /// The time the sequencer created this block before executing transactions
    pub block_timestamp: u64,
    /// The number of transactions in a block
    pub transaction_count: u128,
    /// A commitment to the transactions included in the block
    pub transaction_commitment: Felt,
    /// The number of events
    pub event_count: u128,
    /// A commitment to the events produced in this block
    pub event_commitment: Felt,
    /// The version of the Starknet protocol used when creating this block
    pub protocol_version: StarknetVersion,
    /// Gas prices for this block
    pub l1_gas_price: GasPrices,
    /// The mode of data availability for this block
    pub l1_da_mode: L1DataAvailabilityMode,
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct GasPrices {
    pub eth_l1_gas_price: u128,
    pub strk_l1_gas_price: u128,
    pub eth_l1_data_gas_price: u128,
    pub strk_l1_data_gas_price: u128,
}

impl From<&GasPrices> for blockifier::block::GasPrices {
    fn from(gas_prices: &GasPrices) -> Self {
        let one = NonZeroU128::new(1).unwrap();

        Self {
            eth_l1_gas_price: NonZeroU128::new(gas_prices.eth_l1_gas_price).unwrap_or(one),
            strk_l1_gas_price: NonZeroU128::new(gas_prices.strk_l1_gas_price).unwrap_or(one),
            eth_l1_data_gas_price: NonZeroU128::new(gas_prices.eth_l1_data_gas_price).unwrap_or(one),
            strk_l1_data_gas_price: NonZeroU128::new(gas_prices.strk_l1_data_gas_price).unwrap_or(one),
        }
    }
}

impl GasPrices {
    pub fn l1_gas_price(&self) -> starknet_core::types::ResourcePrice {
        starknet_core::types::ResourcePrice {
            price_in_fri: self.strk_l1_gas_price.into(),
            price_in_wei: self.eth_l1_gas_price.into(),
        }
    }
    pub fn l1_data_gas_price(&self) -> starknet_core::types::ResourcePrice {
        starknet_core::types::ResourcePrice {
            price_in_fri: self.strk_l1_data_gas_price.into(),
            price_in_wei: self.eth_l1_data_gas_price.into(),
        }
    }
}

#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum L1DataAvailabilityMode {
    #[default]
    Calldata,
    Blob,
}

impl From<L1DataAvailabilityMode> for starknet_core::types::L1DataAvailabilityMode {
    fn from(value: L1DataAvailabilityMode) -> Self {
        match value {
            L1DataAvailabilityMode::Calldata => Self::Calldata,
            L1DataAvailabilityMode::Blob => Self::Blob,
        }
    }
}

const BLOCKIFIER_VERSIONED_CONSTANTS_JSON_0_13_0: &[u8] = include_bytes!("../resources/versioned_constants_13_0.json");
const BLOCKIFIER_VERSIONED_CONSTANTS_JSON_0_13_1: &[u8] = include_bytes!("../resources/versioned_constants_13_1.json");

lazy_static::lazy_static! {
pub static ref BLOCKIFIER_VERSIONED_CONSTANTS_0_13_0: VersionedConstants =
    serde_json::from_slice(BLOCKIFIER_VERSIONED_CONSTANTS_JSON_0_13_0).unwrap();

pub static ref BLOCKIFIER_VERSIONED_CONSTANTS_0_13_1: VersionedConstants =
    serde_json::from_slice(BLOCKIFIER_VERSIONED_CONSTANTS_JSON_0_13_1).unwrap();
}

#[derive(thiserror::Error, Debug)]
pub enum BlockFormatError {
    #[error("The block is a pending block")]
    BlockIsPending,
    #[error("Unsupported protocol version")]
    UnsupportedBlockVersion,
}

impl Header {
    /// Creates a new header.
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn new(
        parent_block_hash: Felt,
        block_number: u64,
        global_state_root: Felt,
        sequencer_address: Felt,
        block_timestamp: u64,
        transaction_count: u128,
        transaction_commitment: Felt,
        event_count: u128,
        event_commitment: Felt,
        protocol_version: StarknetVersion,
        gas_prices: GasPrices,
        l1_da_mode: L1DataAvailabilityMode,
    ) -> Self {
        Self {
            parent_block_hash,
            block_number,
            global_state_root,
            sequencer_address,
            block_timestamp,
            transaction_count,
            transaction_commitment,
            event_count,
            event_commitment,
            protocol_version,
            l1_gas_price: gas_prices,
            l1_da_mode,
        }
    }

    /// Compute the hash of the header.
    pub fn hash(&self, chain_id: Felt) -> Felt {
        if self.block_number < 833 && chain_id == MAIN_CHAIN_ID {
            Pedersen::hash_array(&[
                Felt::from(self.block_number),
                self.global_state_root,
                Felt::ZERO,
                Felt::ZERO,
                Felt::from(self.transaction_count),
                self.transaction_commitment,
                Felt::ZERO,
                Felt::ZERO,
                Felt::ZERO,
                Felt::ZERO,
                Felt::from_bytes_be_slice(b"SN_MAIN"),
                self.parent_block_hash,
            ])
        } else {
            // Computes the block hash for blocks generated after Cairo 0.7.0
            Pedersen::hash_array(&[
                Felt::from(self.block_number),      // block number
                self.global_state_root,             // global state root
                self.sequencer_address,             // sequencer address
                Felt::from(self.block_timestamp),   // block timestamp
                Felt::from(self.transaction_count), // number of transactions
                self.transaction_commitment,        // transaction commitment
                Felt::from(self.event_count),       // number of events
                self.event_commitment,              // event commitment
                Felt::ZERO,                         // reserved: protocol version
                Felt::ZERO,                         // reserved: extra data
                self.parent_block_hash,             // parent block hash
            ])
        }
    }
}
