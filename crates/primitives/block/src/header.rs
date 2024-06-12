use core::num::NonZeroU128;

use blockifier::block::{BlockInfo, GasPrices};
use blockifier::context::{BlockContext, ChainInfo, FeeTokenAddresses};
use blockifier::versioned_constants::VersionedConstants;
use mp_convert::core_felt::CoreFelt;
use primitive_types::U256;
use starknet_api::block::{BlockNumber, BlockTimestamp};
use starknet_api::core::{ChainId, ContractAddress};
use starknet_api::data_availability::L1DataAvailabilityMode;
use starknet_api::hash::StarkHash;
use starknet_types_core::felt::Felt;
use starknet_types_core::hash::Pedersen;
use starknet_types_core::hash::StarkHash as StarkHashTrait;

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
/// Starknet header definition.
pub struct Header {
    /// The hash of this block’s parent.
    pub parent_block_hash: StarkHash,
    /// The number (height) of this block.
    pub block_number: u64,
    /// The state commitment after this block.
    pub global_state_root: StarkHash,
    /// The Starknet address of the sequencer who created this block.
    pub sequencer_address: ContractAddress,
    /// The time the sequencer created this block before executing transactions
    pub block_timestamp: u64,
    /// The number of transactions in a block
    pub transaction_count: u128,
    /// A commitment to the transactions included in the block
    pub transaction_commitment: StarkHash,
    /// The number of events
    pub event_count: u128,
    /// A commitment to the events produced in this block
    pub event_commitment: StarkHash,
    /// The version of the Starknet protocol used when creating this block
    pub protocol_version: String, // TODO: Verify if the type can be changed to u8 for the protocol version
    /// Gas prices for this block
    pub l1_gas_price: Option<GasPrices>,
    /// The mode of data availability for this block
    pub l1_da_mode: L1DataAvailabilityMode,
    /// Extraneous data that might be useful for running transactions
    pub extra_data: Option<U256>,
}

const BLOCKIFIER_VERSIONED_CONSTANTS_JSON_0_13_0: &[u8] = include_bytes!("../resources/versioned_constants_13_0.json");

const BLOCKIFIER_VERSIONED_CONSTANTS_JSON_0_13_1: &[u8] = include_bytes!("../resources/versioned_constants_13_1.json");

lazy_static::lazy_static! {
pub static ref BLOCKIFIER_VERSIONED_CONSTANTS_0_13_0: VersionedConstants =
    serde_json::from_slice(BLOCKIFIER_VERSIONED_CONSTANTS_JSON_0_13_0).unwrap();

pub static ref BLOCKIFIER_VERSIONED_CONSTANTS_0_13_1: VersionedConstants =
    serde_json::from_slice(BLOCKIFIER_VERSIONED_CONSTANTS_JSON_0_13_1).unwrap();
}

impl Header {
    /// Creates a new header.
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn new(
        parent_block_hash: StarkHash,
        block_number: u64,
        global_state_root: StarkHash,
        sequencer_address: ContractAddress,
        block_timestamp: u64,
        transaction_count: u128,
        transaction_commitment: StarkHash,
        event_count: u128,
        event_commitment: StarkHash,
        protocol_version: String,
        gas_prices: Option<GasPrices>,
        l1_da_mode: L1DataAvailabilityMode,
        extra_data: Option<U256>,
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
            extra_data,
        }
    }

    /// Converts to a blockifier BlockContext
    pub fn into_block_context(&self, fee_token_addresses: FeeTokenAddresses, chain_id: ChainId) -> BlockContext {
        let versioned_constants = if self.block_number <= 607_877 {
            &BLOCKIFIER_VERSIONED_CONSTANTS_0_13_0
        } else if self.block_number <= 632_914 {
            &BLOCKIFIER_VERSIONED_CONSTANTS_0_13_1
        } else {
            VersionedConstants::latest_constants()
        };

        let chain_info = ChainInfo { chain_id, fee_token_addresses };

        let block_info = BlockInfo {
            block_number: BlockNumber(self.block_number),
            block_timestamp: BlockTimestamp(self.block_timestamp),
            sequencer_address: self.sequencer_address,
            gas_prices: self.l1_gas_price.clone().unwrap_or(GasPrices {
                eth_l1_gas_price: NonZeroU128::new(1).unwrap(),
                strk_l1_gas_price: NonZeroU128::new(1).unwrap(),
                eth_l1_data_gas_price: NonZeroU128::new(1).unwrap(),
                strk_l1_data_gas_price: NonZeroU128::new(1).unwrap(),
            }),
            // TODO: Verify if this is correct
            use_kzg_da: self.l1_da_mode == L1DataAvailabilityMode::Blob,
        };

        BlockContext::new_unchecked(&block_info, &chain_info, versioned_constants)
    }

    /// Compute the hash of the header.
    pub fn hash(&self) -> Felt {
        if self.block_number >= 833 {
            // Computes the block hash for blocks generated after Cairo 0.7.0
            let data: &[Felt] = &[
                Felt::from(self.block_number),                // block number
                self.global_state_root.into_core_felt(),      // global state root
                self.sequencer_address.into_core_felt(),      // sequencer address
                Felt::from(self.block_timestamp),             // block timestamp
                Felt::from(self.transaction_count),           // number of transactions
                self.transaction_commitment.into_core_felt(), // transaction commitment
                Felt::from(self.event_count),                 // number of events
                self.event_commitment.into_core_felt(),       // event commitment
                Felt::ZERO,                                   // reserved: protocol version
                Felt::ZERO,                                   // reserved: extra data
                self.parent_block_hash.into_core_felt(),      // parent block hash
            ];

            Pedersen::hash_array(data)
        } else {
            // Computes the block hash for blocks generated before Cairo 0.7.0
            let data: &[Felt] = &[
                Felt::from(self.block_number),
                self.global_state_root.into_core_felt(),
                Felt::ZERO,
                Felt::ZERO,
                Felt::from(self.transaction_count),
                self.transaction_commitment.into_core_felt(),
                Felt::ZERO,
                Felt::ZERO,
                Felt::ZERO,
                Felt::ZERO,
                Felt::from_bytes_be_slice(b"SN_MAIN"),
                self.parent_block_hash.into_core_felt(),
            ];

            Pedersen::hash_array(data)
        }
    }
}
