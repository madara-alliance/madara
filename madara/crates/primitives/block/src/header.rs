use mp_chain_config::L1DataAvailabilityMode;
use mp_chain_config::StarknetVersion;
use serde::Deserialize;
use serde::Serialize;
use starknet_types_core::felt::Felt;
use starknet_types_core::hash::Pedersen;
use starknet_types_core::hash::Poseidon;
use starknet_types_core::hash::StarkHash as StarkHashTrait;
use std::fmt;
use std::time::SystemTime;

/// Block status.
///
/// The status of the block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BlockStatus {
    Pending,
    #[default]
    AcceptedOnL2,
    AcceptedOnL1,
    Rejected,
}

impl From<BlockStatus> for mp_rpc::BlockStatus {
    fn from(status: BlockStatus) -> Self {
        match status {
            BlockStatus::Pending => mp_rpc::BlockStatus::Pending,
            BlockStatus::AcceptedOnL2 => mp_rpc::BlockStatus::AcceptedOnL2,
            BlockStatus::AcceptedOnL1 => mp_rpc::BlockStatus::AcceptedOnL1,
            BlockStatus::Rejected => mp_rpc::BlockStatus::Rejected,
        }
    }
}

#[derive(Clone, Copy, Default, Debug, PartialEq, PartialOrd, Ord, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BlockTimestamp(pub u64);
impl BlockTimestamp {
    pub fn now() -> Self {
        Self(
            SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).expect("SystemTime::now() < Unix epoch").as_secs(),
        )
    }
}

impl From<u64> for BlockTimestamp {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

impl From<SystemTime> for BlockTimestamp {
    fn from(value: SystemTime) -> Self {
        Self(value.duration_since(SystemTime::UNIX_EPOCH).expect("SystemTime::now() < Unix epoch").as_secs())
    }
}

impl fmt::Display for BlockTimestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingHeader {
    /// The hash of this block’s parent.
    pub parent_block_hash: Felt,
    /// The Starknet address of the sequencer who created this block.
    pub sequencer_address: Felt,
    /// Unix timestamp (seconds) when the block was produced -- before executing any transaction.
    pub block_timestamp: BlockTimestamp,
    /// The version of the Starknet protocol used when creating this block
    pub protocol_version: StarknetVersion,
    /// Gas prices for this block
    pub gas_prices: GasPrices,
    /// The mode of data availability for this block
    pub l1_da_mode: L1DataAvailabilityMode,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Unix timestamp (seconds) when the block was produced -- before executing any transaction.
    pub block_timestamp: BlockTimestamp,
    /// The number of transactions in a block
    pub transaction_count: u64,
    /// A commitment to the transactions included in the block
    pub transaction_commitment: Felt,
    /// The number of events
    pub event_count: u64,
    /// A commitment to the events produced in this block
    pub event_commitment: Felt,
    /// The number of state diff elements
    pub state_diff_length: Option<u64>,
    /// A commitment to the state diff elements
    pub state_diff_commitment: Option<Felt>,
    /// A commitment to the receipts produced in this block
    pub receipt_commitment: Option<Felt>,
    /// The version of the Starknet protocol used when creating this block
    pub protocol_version: StarknetVersion,
    /// Gas prices for this block
    pub gas_prices: GasPrices,
    /// The mode of data availability for this block
    pub l1_da_mode: L1DataAvailabilityMode,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GasPrices {
    pub eth_l1_gas_price: u128,
    pub strk_l1_gas_price: u128,
    pub eth_l1_data_gas_price: u128,
    pub strk_l1_data_gas_price: u128,
    pub eth_l2_gas_price: u128,
    pub strk_l2_gas_price: u128,
}

// Starknet API can't have null gas prices, so the default null gas prices are set to 1.
impl From<&GasPrices> for starknet_api::block::GasPrices {
    fn from(gas_prices: &GasPrices) -> Self {
        Self {
            eth_gas_prices: starknet_api::block::GasPriceVector {
                l1_gas_price: starknet_api::block::NonzeroGasPrice::new(gas_prices.eth_l1_gas_price.into())
                    .unwrap_or_default(),
                l1_data_gas_price: starknet_api::block::NonzeroGasPrice::new(gas_prices.eth_l1_data_gas_price.into())
                    .unwrap_or_default(),
                l2_gas_price: starknet_api::block::NonzeroGasPrice::new(gas_prices.eth_l2_gas_price.into())
                    .unwrap_or_default(),
            },
            strk_gas_prices: starknet_api::block::GasPriceVector {
                l1_gas_price: starknet_api::block::NonzeroGasPrice::new(gas_prices.strk_l1_gas_price.into())
                    .unwrap_or_default(),
                l1_data_gas_price: starknet_api::block::NonzeroGasPrice::new(gas_prices.strk_l1_data_gas_price.into())
                    .unwrap_or_default(),
                l2_gas_price: starknet_api::block::NonzeroGasPrice::new(gas_prices.strk_l2_gas_price.into())
                    .unwrap_or_default(),
            },
        }
    }
}

impl GasPrices {
    pub fn l1_gas_price(&self) -> mp_rpc::ResourcePrice {
        mp_rpc::ResourcePrice {
            price_in_fri: self.strk_l1_gas_price.into(),
            price_in_wei: self.eth_l1_gas_price.into(),
        }
    }

    pub fn l1_data_gas_price(&self) -> mp_rpc::ResourcePrice {
        mp_rpc::ResourcePrice {
            price_in_fri: self.strk_l1_data_gas_price.into(),
            price_in_wei: self.eth_l1_data_gas_price.into(),
        }
    }

    /// https://docs.starknet.io/architecture/blocks/#block_hash
    pub fn compute_hash(&self) -> Felt {
        println!("Computing GasPrices hash is called here!");
        Poseidon::hash_array(&[
            Felt::from_bytes_be_slice(b"STARKNET_GAS_PRICES0"),
            Felt::from(self.eth_l1_gas_price),
            Felt::from(self.strk_l1_gas_price),
            Felt::from(self.eth_l1_data_gas_price),
            Felt::from(self.strk_l1_data_gas_price),
            Felt::from(self.eth_l2_gas_price),
            Felt::from(self.strk_l2_gas_price),
        ])
    }
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
        block_timestamp: BlockTimestamp,
        transaction_count: u64,
        transaction_commitment: Felt,
        event_count: u64,
        event_commitment: Felt,
        state_diff_length: Option<u64>,
        state_diff_commitment: Option<Felt>,
        receipt_commitment: Option<Felt>,
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
            state_diff_length,
            state_diff_commitment,
            receipt_commitment,
            protocol_version,
            gas_prices,
            l1_da_mode,
        }
    }

    /// Compute the hash of the header.
    /// https://docs.starknet.io/architecture/blocks/#block_hash
    pub fn compute_hash(&self, chain_id: Felt, pre_v0_13_2_override: bool) -> Felt {
        println!("Computing Block hash is called here!");
        let hash_version = if self.protocol_version < StarknetVersion::V0_13_2 && pre_v0_13_2_override {
            StarknetVersion::V0_13_2
        } else {
            self.protocol_version
        };

        if hash_version.is_pre_v0_7() {
            println!("Computing Block hash compute_hash_inner_pre_v0_7");
            self.compute_hash_inner_pre_v0_7(chain_id)
        } else if hash_version < StarknetVersion::V0_13_2 {
            println!("Computing Block hash ");
            Pedersen::hash_array(&[
                Felt::from(self.block_number),
                self.global_state_root,
                self.sequencer_address,
                Felt::from(self.block_timestamp.0),
                Felt::from(self.transaction_count),
                self.transaction_commitment,
                Felt::from(self.event_count),
                self.event_commitment,
                Felt::ZERO, // reserved: protocol version
                Felt::ZERO, // reserved: extra data
                self.parent_block_hash,
            ])
        } else if hash_version < StarknetVersion::V0_13_4 {
            println!("Computing Block hash compute_hash_inner_v0");
            self.compute_hash_inner_v0()
        } else {
            println!("Computing Block hash compute_hash_inner_v1");
            self.compute_hash_inner_v1()
        }
    }

    fn compute_hash_inner_pre_v0_7(&self, chain_id: Felt) -> Felt {
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
            chain_id,
            self.parent_block_hash,
        ])
    }

    fn compute_hash_inner_v0(&self) -> Felt {
        println!("compute_hash_inner_v0 debug:");
        println!("  block_number: {:?}", self.block_number);
        println!("  global_state_root: {:?}", self.global_state_root);
        println!("  sequencer_address: {:?}", self.sequencer_address);
        println!("  block_timestamp.0: {:?}", self.block_timestamp.0);
        println!("  transaction_count: {:?}", self.transaction_count);
        println!("  event_count: {:?}", self.event_count);
        println!("  state_diff_length: {:?}", self.state_diff_length);
        println!("  l1_da_mode: {:?}", self.l1_da_mode);
        println!("  state_diff_commitment: {:?}", self.state_diff_commitment);
        println!("  transaction_commitment: {:?}", self.transaction_commitment);
        println!("  event_commitment: {:?}", self.event_commitment);
        println!("  receipt_commitment: {:?}", self.receipt_commitment);
        println!("  gas_prices.eth_l1_gas_price: {:?}", self.gas_prices.eth_l1_gas_price);
        println!("  gas_prices.strk_l1_gas_price: {:?}", self.gas_prices.strk_l1_gas_price);
        println!("  gas_prices.eth_l1_data_gas_price: {:?}", self.gas_prices.eth_l1_data_gas_price);
        println!("  gas_prices.strk_l1_data_gas_price: {:?}", self.gas_prices.strk_l1_data_gas_price);
        println!("  protocol_version: {:?}", self.protocol_version);
        println!("  parent_block_hash: {:?}", self.parent_block_hash);

        // Also print the computed values used in the hash
        let timestamp_felt = Felt::from(self.block_timestamp.0);
        let concat_counts_result = concat_counts(
            self.transaction_count,
            self.event_count,
            self.state_diff_length.unwrap_or(0),
            self.l1_da_mode,
        );
        let protocol_version_felt = Felt::from_bytes_be_slice(self.protocol_version.to_string().as_bytes());

        println!("  Computed values:");
        println!("  timestamp_felt: {:?}", timestamp_felt);
        println!("  concat_counts_result: {:?}", concat_counts_result);
        println!("  state_diff_length.unwrap_or(0): {:?}", self.state_diff_length.unwrap_or(0));
        println!("  state_diff_commitment.unwrap_or(Felt::ZERO): {:?}", self.state_diff_commitment.unwrap_or(Felt::ZERO));
        println!("  receipt_commitment.unwrap_or(Felt::ZERO): {:?}", self.receipt_commitment.unwrap_or(Felt::ZERO));
        println!("  protocol_version_felt: {:?}", protocol_version_felt);

        let block_hash = Poseidon::hash_array(&[
            Felt::from_bytes_be_slice(b"STARKNET_BLOCK_HASH0"),
            Felt::from(self.block_number),
            self.global_state_root,
            self.sequencer_address,
            timestamp_felt,
            concat_counts_result,
            self.state_diff_commitment.unwrap_or(Felt::ZERO),
            self.transaction_commitment,
            self.event_commitment,
            self.receipt_commitment.unwrap_or(Felt::ZERO),
            self.gas_prices.eth_l1_gas_price.into(),
            self.gas_prices.strk_l1_gas_price.into(),
            self.gas_prices.eth_l1_data_gas_price.into(),
            self.gas_prices.strk_l1_data_gas_price.into(),
            protocol_version_felt,
            Felt::ZERO,
            self.parent_block_hash,
        ]);
        println!("  block_hash: {:?}", block_hash);
        block_hash
    }

    fn compute_hash_inner_v1(&self) -> Felt {
        Poseidon::hash_array(&[
            Felt::from_bytes_be_slice(b"STARKNET_BLOCK_HASH1"),
            Felt::from(self.block_number),
            self.global_state_root,
            self.sequencer_address,
            Felt::from(self.block_timestamp.0),
            concat_counts(
                self.transaction_count,
                self.event_count,
                self.state_diff_length.unwrap_or(0),
                self.l1_da_mode,
            ),
            self.state_diff_commitment.unwrap_or(Felt::ZERO),
            self.transaction_commitment,
            self.event_commitment,
            self.receipt_commitment.unwrap_or(Felt::ZERO),
            self.gas_prices.compute_hash(),
            Felt::from_bytes_be_slice(self.protocol_version.to_string().as_bytes()),
            Felt::ZERO,
            self.parent_block_hash,
        ])
    }
}

fn concat_counts(
    transaction_count: u64,
    event_count: u64,
    state_diff_length: u64,
    l1_da_mode: L1DataAvailabilityMode,
) -> Felt {
    let l1_data_availability_byte: u8 = match l1_da_mode {
        L1DataAvailabilityMode::Calldata => 0,
        L1DataAvailabilityMode::Blob => 0b10000000,
    };

    let concat_bytes = [
        transaction_count.to_be_bytes(),
        event_count.to_be_bytes(),
        state_diff_length.to_be_bytes(),
        [l1_data_availability_byte, 0_u8, 0_u8, 0_u8, 0_u8, 0_u8, 0_u8, 0_u8],
    ]
    .concat();

    Felt::from_bytes_be_slice(concat_bytes.as_slice())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_concat_counts() {
        let concated = concat_counts(4, 3, 2, L1DataAvailabilityMode::Blob);
        assert_eq!(
            concated,
            Felt::from_hex_unchecked("0x0000000000000004000000000000000300000000000000028000000000000000")
        );
    }

    #[test]
    fn test_header_new() {
        let header = Header::new(
            Felt::from(1),
            2,
            Felt::from(3),
            Felt::from(4),
            BlockTimestamp(5),
            6,
            Felt::from(7),
            8,
            Felt::from(9),
            Some(10),
            Some(Felt::from(11)),
            Some(Felt::from(12)),
            "0.13.2".parse().unwrap(),
            GasPrices {
                eth_l1_gas_price: 14,
                strk_l1_gas_price: 15,
                eth_l1_data_gas_price: 16,
                strk_l1_data_gas_price: 17,
                ..Default::default()
            },
            L1DataAvailabilityMode::Blob,
        );

        assert_eq!(header, dummy_header(StarknetVersion::V0_13_2));
    }

    #[test]
    fn test_header_hash_v0_13_2() {
        let header = dummy_header(StarknetVersion::V0_13_2);
        let hash = header.compute_hash(Felt::from_bytes_be_slice(b"CHAIN_ID"), false);
        let expected_hash =
            Felt::from_hex_unchecked("0x545dd9ef652b07cebb3c8b6d43b6c477998f124e75df970dfee300fb32a698b");
        assert_eq!(hash, expected_hash);
    }

    #[test]
    fn test_header_hash_v0_11_1() {
        let header = dummy_header(StarknetVersion::V0_11_1);
        let hash = header.compute_hash(Felt::from_bytes_be_slice(b"CHAIN_ID"), false);
        let expected_hash =
            Felt::from_hex_unchecked("0x42ec5792c165e0235d7576dc9b4a56140b217faba0b2f57c0a48b850ea5999c");
        assert_eq!(hash, expected_hash);
    }

    #[test]
    fn test_header_hash_pre_v0_7() {
        let header = dummy_header(StarknetVersion::V_0_0_0);
        let hash = header.compute_hash(Felt::from_bytes_be_slice(b"SN_MAIN"), false);
        let expected_hash =
            Felt::from_hex_unchecked("0x6028bf0975e1d4c95713e021a0f0217e74d5a748a20691d881c86d9d62d1432");
        assert_eq!(hash, expected_hash);
    }

    fn dummy_header(protocol_version: StarknetVersion) -> Header {
        Header {
            parent_block_hash: Felt::from(1),
            block_number: 2,
            global_state_root: Felt::from(3),
            sequencer_address: Felt::from(4),
            block_timestamp: BlockTimestamp(5),
            transaction_count: 6,
            transaction_commitment: Felt::from(7),
            event_count: 8,
            event_commitment: Felt::from(9),
            state_diff_length: Some(10),
            state_diff_commitment: Some(Felt::from(11)),
            receipt_commitment: Some(Felt::from(12)),
            protocol_version,
            gas_prices: GasPrices {
                eth_l1_gas_price: 14,
                strk_l1_gas_price: 15,
                eth_l1_data_gas_price: 16,
                strk_l1_data_gas_price: 17,
                ..Default::default()
            },
            l1_da_mode: L1DataAvailabilityMode::Blob,
        }
    }
}
