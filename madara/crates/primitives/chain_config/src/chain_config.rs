//! Note: We are NOT using fs read for constants, as they NEED to be included in the resulting
//! binary. Otherwise, using the madara binary without cloning the repo WILL crash, and that's very very bad.
//! The binary needs to be self contained! We need to be able to ship madara as a single binary, without
//! the user needing to clone the repo.
//! Only use `fs` for constants when writing tests.

use crate::{L1DataAvailabilityMode, StarknetVersion};
use anyhow::{bail, Context, Result};
use blockifier::blockifier::config::ConcurrencyConfig;
use blockifier::blockifier_versioned_constants::{RawVersionedConstants, VersionedConstants};
use blockifier::bouncer::BouncerConfig;
use blockifier::context::{ChainInfo, FeeTokenAddresses};
use lazy_static::__Deref;
use mp_utils::crypto::ZeroingPrivateKey;
use mp_utils::serde::{deserialize_duration, deserialize_optional_duration};
use serde::de::{MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use starknet_api::core::{ChainId, ContractAddress, PatriciaKey};
use starknet_types_core::felt::Felt;
use std::fmt;
use std::str::FromStr;
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::Read,
    path::Path,
    time::Duration,
};
use url::Url;

/// Custom serde module for u128 values in YAML
/// serde_yaml doesn't natively support u128, so we handle both numbers and strings
mod serde_u128 {
    use serde::{Deserializer, Serializer};

    pub fn serialize<S>(value: &u128, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_u128(*value)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<u128, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct U128Visitor;

        impl<'de> serde::de::Visitor<'de> for U128Visitor {
            type Value = u128;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a u128 as a number or string")
            }

            fn visit_i64<E>(self, value: i64) -> Result<u128, E>
            where
                E: serde::de::Error,
            {
                if value < 0 {
                    return Err(E::custom("u128 cannot be negative"));
                }
                Ok(value as u128)
            }

            fn visit_u64<E>(self, value: u64) -> Result<u128, E>
            where
                E: serde::de::Error,
            {
                Ok(value as u128)
            }

            fn visit_str<E>(self, value: &str) -> Result<u128, E>
            where
                E: serde::de::Error,
            {
                value.parse::<u128>().map_err(E::custom)
            }
        }

        deserializer.deserialize_any(U128Visitor)
    }
}

pub mod eth_core_contract_address {
    pub const MAINNET: &str = "0xc662c410C0ECf747543f5bA90660f6ABeBD9C8c4";
    pub const SEPOLIA_TESTNET: &str = "0xE2Bb56ee936fd6433DC0F6e7e3b8365C906AA057";
    pub const SEPOLIA_INTEGRATION: &str = "0x4737c0c1B4D5b1A687B42610DdabEE781152359c";
}

pub mod eth_gps_statement_verifier {
    pub const MAINNET: &str = "0x47312450B3Ac8b5b8e247a6bB6d523e7605bDb60";
    pub const SEPOLIA_TESTNET: &str = "0xf294781D719D2F4169cE54469C28908E6FA752C1";
    pub const SEPOLIA_INTEGRATION: &str = "0x2046B966994Adcb88D83f467a41b75d64C2a619F";
}

pub mod public_key {
    pub const MAINNET: &str = "0x48253ff2c3bed7af18bde0b611b083b39445959102d4947c51c4db6aa4f4e58";
    pub const SEPOLIA_TESTNET: &str = "0x1252b6bce1351844c677869c6327e80eae1535755b611c66b8f46e595b40eea";
    pub const SEPOLIA_INTEGRATION: &str = "0x4e4856eb36dbd5f4a7dca29f7bb5232974ef1fb7eb5b597c58077174c294da1";
}

/// Current chain config version
pub const CURRENT_CONFIG_VERSION: u32 = 1;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct BlockProductionConfig {
    /// Disable optimistic parallel execution.
    pub disable_concurrency: bool,
    /// Number of workers. Defaults to the number of cores in the system.
    pub n_workers: usize,
    pub batch_size: usize,
}

impl BlockProductionConfig {
    pub fn blockifier_config(&self) -> ConcurrencyConfig {
        ConcurrencyConfig { enabled: !self.disable_concurrency, n_workers: self.n_workers, chunk_size: self.batch_size }
    }
}

impl Default for BlockProductionConfig {
    fn default() -> Self {
        Self {
            disable_concurrency: false,
            n_workers: std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1),
            batch_size: 1024,
        }
    }
}

fn starknet_version_latest() -> StarknetVersion {
    StarknetVersion::LATEST
}
fn default_block_time() -> Duration {
    Duration::from_secs(30)
}
fn default_l1_messages_replay_max_duration() -> Duration {
    Duration::from_secs(3 * 24 * 60 * 60)
}
fn default_l1_messages_finality_blocks() -> u64 {
    10 // Default: wait for 10 L1 blocks before processing messages (~2 minutes on Ethereum)
}
fn default_mempool_min_tip_bump() -> f64 {
    0.1
}

#[derive(thiserror::Error, Debug)]
#[error("Unsupported protocol version: {0}")]
pub struct UnsupportedProtocolVersion(StarknetVersion);

#[derive(Debug, Serialize, Deserialize, Default, Clone, Copy)]
pub enum MempoolMode {
    #[default]
    Timestamp,
    Tip,
}

#[derive(Debug, Serialize, Deserialize, Default, Clone, Copy, PartialEq, Eq)]
pub enum SettlementChainKind {
    #[default]
    Ethereum,
    Starknet,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum L2GasPrice {
    /// Fixed L2 gas price
    Fixed {
        /// The fixed gas price value
        #[serde(with = "serde_u128")]
        price: u128,
    },
    /// EIP-1559 style dynamic gas pricing
    #[serde(rename = "eip1559")]
    EIP1559 {
        /// The target gas usage per block for the block production
        #[serde(with = "serde_u128")]
        target: u128,
        /// The minimum l2 gas price
        #[serde(with = "serde_u128")]
        min_price: u128,
        /// The maximum change in l2 gas price per block (EIP-1559)
        #[serde(with = "serde_u128")]
        max_change_denominator: u128,
    },
}

impl L2GasPrice {
    fn starknet_mainnet() -> Self {
        Self::EIP1559 { target: 2_000_000_000, min_price: 100_000, max_change_denominator: 48 }
    }
}

/// Chain config version 1 structure (without config_version field - it's in the enum tag)
#[derive(Debug, Deserialize, Serialize)]
pub struct ChainConfigV1 {
    /// Human-readable chain name, for displaying to the console.
    pub chain_name: String,
    pub chain_id: ChainId,

    /// The DA mode supported by L1.
    #[serde(default)]
    pub l1_da_mode: L1DataAvailabilityMode,

    #[serde(default)]
    pub settlement_chain_kind: SettlementChainKind,

    // The Gateway URLs are the URLs of the endpoint that the node will use to sync blocks in full mode.
    pub feeder_gateway_url: Url,
    pub gateway_url: Url,

    /// For starknet, this is the STRK ERC-20 contract on starknet.
    pub native_fee_token_address: ContractAddress,
    /// For starknet, this is the ETH ERC-20 contract on starknet.
    pub parent_fee_token_address: ContractAddress,

    #[serde(default, skip)]
    pub versioned_constants: ChainVersionedConstants,

    /// Produce blocks using for this starknet protocol version.
    #[serde(
        default = "starknet_version_latest",
        deserialize_with = "deserialize_starknet_version",
        serialize_with = "serialize_starknet_version"
    )]
    pub latest_protocol_version: StarknetVersion,

    /// Only used for block production.
    /// Default: 30s.
    #[serde(
        default = "default_block_time",
        deserialize_with = "deserialize_duration",
        serialize_with = "serialize_duration"
    )]
    pub block_time: Duration,

    /// Do not produce empty blocks.
    /// Warning: If a chain does not produce blocks regularily, estimate_fee RPC may behave incorrectly as its gas prices
    /// are based on the latest block on chain.
    #[serde(default)]
    pub no_empty_blocks: bool,

    /// Only used for block production.
    /// The bouncer is in charge of limiting block sizes. This is where the max number of step per block, gas etc are.
    #[serde(default)]
    pub bouncer_config: BouncerConfig,

    /// Only used for block production.
    pub sequencer_address: ContractAddress,

    /// The Starknet core contract address for the L1 watcher.
    pub eth_core_contract_address: String,

    /// The Starknet SHARP verifier L1 address. Check out the [docs](https://docs.starknet.io/architecture-and-concepts/solidity-verifier/)
    /// for more information
    pub eth_gps_statement_verifier: String,

    /// Private key used by the node to sign blocks provided through the
    /// feeder gateway. This serves as a proof of origin and in the future
    /// will also be used by the p2p protocol and tendermint consensus.
    /// > [!NOTE]
    /// > This key will be auto-generated on startup if none is provided.
    /// > This also means the private key is by default regenerated on boot
    #[serde(skip)]
    pub private_key: Option<ZeroingPrivateKey>,

    #[serde(default)]
    pub mempool_mode: MempoolMode,
    /// Minimum tip increase when replacing a transaction with the same (contract_address, nonce) pair in the mempool, as a ratio.
    /// Tip bumping allows users to increase the priority of their transaction in the mempool, so that they are included in a block sooner.
    /// This has no effect on FCFS (First-come-first-serve) mode mempools.
    /// Default is 0.1 which means you have to increase the tip by at least 10%.
    #[serde(default = "default_mempool_min_tip_bump")]
    pub mempool_min_tip_bump: f64,
    /// Transaction limit in the mempool.
    pub mempool_max_transactions: usize,
    /// Transaction limit in the mempool, we have an additional limit for declare transactions.
    pub mempool_max_declare_transactions: Option<usize>,
    /// Max age of a transaction in the mempool.
    #[serde(deserialize_with = "deserialize_optional_duration", serialize_with = "serialize_optional_duration")]
    pub mempool_ttl: Option<Duration>,
    /// L2 gas price configuration - either fixed or EIP-1559 dynamic pricing
    pub l2_gas_price: L2GasPrice,

    /// Configuration for parallel execution in Blockifier. Only used for block production.
    #[serde(default)]
    pub block_production_concurrency: BlockProductionConfig,

    /// Configuration for l1 messages max replay duration.
    #[serde(
        default = "default_l1_messages_replay_max_duration",
        deserialize_with = "deserialize_duration",
        serialize_with = "serialize_duration"
    )]
    pub l1_messages_replay_max_duration: Duration,

    /// Number of L1 blocks to wait before considering a message finalized.
    /// This provides protection against L1 chain reorganizations.
    /// Default: 0 (rely on L1's "finalized" block tag).
    /// Recommended: 12 for Ethereum mainnet (~2.5 minutes).
    #[serde(default = "default_l1_messages_finality_blocks")]
    pub l1_messages_finality_blocks: u64,
}

/// Versioned chain config enum that handles different config versions
#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "config_version")]
pub enum ChainConfigVersioned {
    #[serde(rename = "1")]
    V1(ChainConfigV1),
}

/// Canonical chain config structure used throughout the codebase
#[derive(Debug, Deserialize, Serialize)]
pub struct ChainConfig {
    /// Human-readable chain name, for displaying to the console.
    pub chain_name: String,
    pub chain_id: ChainId,

    /// Chain config version. This is used to track breaking changes to the chain config format.
    /// Version must be explicitly specified in the config file.
    pub config_version: u32,

    /// The DA mode supported by L1.
    #[serde(default)]
    pub l1_da_mode: L1DataAvailabilityMode,

    #[serde(default)]
    pub settlement_chain_kind: SettlementChainKind,

    // The Gateway URLs are the URLs of the endpoint that the node will use to sync blocks in full mode.
    pub feeder_gateway_url: Url,
    pub gateway_url: Url,

    /// For starknet, this is the STRK ERC-20 contract on starknet.
    pub native_fee_token_address: ContractAddress,
    /// For starknet, this is the ETH ERC-20 contract on starknet.
    pub parent_fee_token_address: ContractAddress,

    #[serde(default, skip)]
    pub versioned_constants: ChainVersionedConstants,

    /// Produce blocks using for this starknet protocol version.
    #[serde(
        default = "starknet_version_latest",
        deserialize_with = "deserialize_starknet_version",
        serialize_with = "serialize_starknet_version"
    )]
    pub latest_protocol_version: StarknetVersion,

    /// Only used for block production.
    /// Default: 30s.
    #[serde(
        default = "default_block_time",
        deserialize_with = "deserialize_duration",
        serialize_with = "serialize_duration"
    )]
    pub block_time: Duration,

    /// Do not produce empty blocks.
    /// Warning: If a chain does not produce blocks regularily, estimate_fee RPC may behave incorrectly as its gas prices
    /// are based on the latest block on chain.
    #[serde(default)]
    pub no_empty_blocks: bool,

    /// Only used for block production.
    /// The bouncer is in charge of limiting block sizes. This is where the max number of step per block, gas etc are.
    #[serde(default)]
    pub bouncer_config: BouncerConfig,

    /// Only used for block production.
    pub sequencer_address: ContractAddress,

    /// The Starknet core contract address for the L1 watcher.
    pub eth_core_contract_address: String,

    /// The Starknet SHARP verifier L1 address. Check out the [docs](https://docs.starknet.io/architecture-and-concepts/solidity-verifier/)
    /// for more information
    pub eth_gps_statement_verifier: String,

    /// Private key used by the node to sign blocks provided through the
    /// feeder gateway. This serves as a proof of origin and in the future
    /// will also be used by the p2p protocol and tendermint consensus.
    /// > [!NOTE]
    /// > This key will be auto-generated on startup if none is provided.
    /// > This also means the private key is by default regenerated on boot
    #[serde(skip)]
    pub private_key: Option<ZeroingPrivateKey>,

    #[serde(default)]
    pub mempool_mode: MempoolMode,
    /// Minimum tip increase when replacing a transaction with the same (contract_address, nonce) pair in the mempool, as a ratio.
    /// Tip bumping allows users to increase the priority of their transaction in the mempool, so that they are included in a block sooner.
    /// This has no effect on FCFS (First-come-first-serve) mode mempools.
    /// Default is 0.1 which means you have to increase the tip by at least 10%.
    #[serde(default = "default_mempool_min_tip_bump")]
    pub mempool_min_tip_bump: f64,
    /// Transaction limit in the mempool.
    pub mempool_max_transactions: usize,
    /// Transaction limit in the mempool, we have an additional limit for declare transactions.
    pub mempool_max_declare_transactions: Option<usize>,
    /// Max age of a transaction in the mempool.
    #[serde(deserialize_with = "deserialize_optional_duration", serialize_with = "serialize_optional_duration")]
    pub mempool_ttl: Option<Duration>,
    /// L2 gas price configuration - either fixed or EIP-1559 dynamic pricing
    pub l2_gas_price: L2GasPrice,

    /// Configuration for parallel execution in Blockifier. Only used for block production.
    #[serde(default)]
    pub block_production_concurrency: BlockProductionConfig,

    /// Configuration for l1 messages max replay duration.
    #[serde(
        default = "default_l1_messages_replay_max_duration",
        deserialize_with = "deserialize_duration",
        serialize_with = "serialize_duration"
    )]
    pub l1_messages_replay_max_duration: Duration,

    /// Number of L1 blocks to wait before considering a message finalized.
    /// This provides protection against L1 chain reorganizations.
    /// Default: 0 (rely on L1's "finalized" block tag).
    /// Recommended: 12 for Ethereum mainnet (~2.5 minutes).
    #[serde(default = "default_l1_messages_finality_blocks")]
    pub l1_messages_finality_blocks: u64,
}

impl Clone for ChainConfig {
    /// Clones all fields except `private_key` which is set to `None`.
    /// This is intentional: `ZeroingPrivateKey` doesn't implement Clone for security reasons
    /// (to prevent multiple copies of sensitive key material in memory).
    fn clone(&self) -> Self {
        Self {
            chain_name: self.chain_name.clone(),
            chain_id: self.chain_id.clone(),
            config_version: self.config_version,
            l1_da_mode: self.l1_da_mode,
            settlement_chain_kind: self.settlement_chain_kind,
            feeder_gateway_url: self.feeder_gateway_url.clone(),
            gateway_url: self.gateway_url.clone(),
            native_fee_token_address: self.native_fee_token_address,
            parent_fee_token_address: self.parent_fee_token_address,
            versioned_constants: self.versioned_constants.clone(),
            latest_protocol_version: self.latest_protocol_version,
            block_time: self.block_time,
            no_empty_blocks: self.no_empty_blocks,
            bouncer_config: self.bouncer_config.clone(),
            sequencer_address: self.sequencer_address,
            eth_core_contract_address: self.eth_core_contract_address.clone(),
            eth_gps_statement_verifier: self.eth_gps_statement_verifier.clone(),
            private_key: None, // Intentionally not cloned for security
            mempool_mode: self.mempool_mode,
            mempool_min_tip_bump: self.mempool_min_tip_bump,
            mempool_max_transactions: self.mempool_max_transactions,
            mempool_max_declare_transactions: self.mempool_max_declare_transactions,
            mempool_ttl: self.mempool_ttl,
            l2_gas_price: self.l2_gas_price.clone(),
            block_production_concurrency: self.block_production_concurrency.clone(),
            l1_messages_replay_max_duration: self.l1_messages_replay_max_duration,
            l1_messages_finality_blocks: self.l1_messages_finality_blocks,
        }
    }
}

// Conversion implementations for versioned configs

impl TryFrom<ChainConfigV1> for ChainConfig {
    type Error = anyhow::Error;

    fn try_from(v1: ChainConfigV1) -> Result<Self> {
        Ok(ChainConfig {
            config_version: 1,
            chain_name: v1.chain_name,
            chain_id: v1.chain_id,
            l1_da_mode: v1.l1_da_mode,
            settlement_chain_kind: v1.settlement_chain_kind,
            feeder_gateway_url: v1.feeder_gateway_url,
            gateway_url: v1.gateway_url,
            native_fee_token_address: v1.native_fee_token_address,
            parent_fee_token_address: v1.parent_fee_token_address,
            versioned_constants: v1.versioned_constants,
            latest_protocol_version: v1.latest_protocol_version,
            block_time: v1.block_time,
            no_empty_blocks: v1.no_empty_blocks,
            bouncer_config: v1.bouncer_config,
            sequencer_address: v1.sequencer_address,
            eth_core_contract_address: v1.eth_core_contract_address,
            eth_gps_statement_verifier: v1.eth_gps_statement_verifier,
            private_key: v1.private_key,
            mempool_mode: v1.mempool_mode,
            mempool_min_tip_bump: v1.mempool_min_tip_bump,
            mempool_max_transactions: v1.mempool_max_transactions,
            mempool_max_declare_transactions: v1.mempool_max_declare_transactions,
            mempool_ttl: v1.mempool_ttl,
            l2_gas_price: v1.l2_gas_price,
            block_production_concurrency: v1.block_production_concurrency,
            l1_messages_replay_max_duration: v1.l1_messages_replay_max_duration,
            l1_messages_finality_blocks: v1.l1_messages_finality_blocks,
        })
    }
}

impl TryFrom<ChainConfigVersioned> for ChainConfig {
    type Error = anyhow::Error;

    fn try_from(versioned: ChainConfigVersioned) -> Result<Self> {
        match versioned {
            ChainConfigVersioned::V1(v1) => ChainConfig::try_from(v1),
        }
    }
}

impl ChainConfig {
    pub fn from_yaml(path: &Path) -> Result<Self> {
        let config_str = fs::read_to_string(path)?;
        let config_value: serde_yaml::Value =
            serde_yaml::from_str(&config_str).context("While deserializing chain config")?;

        // Pre-check for config_version field to provide a helpful error message
        // If this field is missing, serde would give a less user-friendly error
        config_value
            .get("config_version")
            .context("Missing required field 'config_version' in chain config. Please add 'config_version: <version>' to your chain config file.")?;

        let versioned_constants_file_paths: BTreeMap<String, String> =
            serde_yaml::from_value(config_value.get("versioned_constants_path").cloned().unwrap_or_default())
                .context("While deserializing versioned constants file paths")?;

        let versioned_constants = {
            // add the defaults VersionedConstants
            let mut versioned_constants = ChainVersionedConstants::default();
            versioned_constants.merge(ChainVersionedConstants::from_file(versioned_constants_file_paths)?);
            versioned_constants
        };

        // Deserialize into versioned enum - serde handles version validation and dispatch
        let versioned: ChainConfigVersioned =
            serde_yaml::from_str(&config_str).context("While deserializing chain config")?;

        // Convert to canonical ChainConfig using TryFrom
        let mut chain_config =
            ChainConfig::try_from(versioned).context("While converting versioned config to ChainConfig")?;

        // Override with loaded versioned constants
        chain_config.versioned_constants = versioned_constants;

        Ok(chain_config)
    }

    /// Verify that the chain config is valid for block production.
    pub fn precheck_block_production(&self) -> Result<()> {
        if self.sequencer_address == ContractAddress::default() {
            bail!("Sequencer address cannot be 0x0 for block production.")
        }
        if self.block_time.is_zero() {
            bail!("Block time cannot be zero for block production.")
        }
        Ok(())
    }

    pub fn starknet_mainnet() -> Self {
        // Sources:
        // - https://docs.starknet.io/tools/important-addresses
        // - https://docs.starknet.io/tools/limits-and-triggers (bouncer & block times)
        // - state_diff_size is the blob size limit of ethereum
        // - bouncer builtin_count, message_segment_length, n_events, state_diff_size are probably wrong
        Self {
            chain_name: "Starknet Mainnet".into(),
            chain_id: ChainId::Mainnet,
            config_version: CURRENT_CONFIG_VERSION,
            // Since L1 here is Ethereum, that supports Blob.
            l1_da_mode: L1DataAvailabilityMode::Blob,
            settlement_chain_kind: SettlementChainKind::Ethereum,
            feeder_gateway_url: Url::parse("https://feeder.alpha-mainnet.starknet.io/feeder_gateway/").unwrap(),
            gateway_url: Url::parse("https://alpha-mainnet.starknet.io/gateway/").unwrap(),
            native_fee_token_address: ContractAddress(
                PatriciaKey::try_from(Felt::from_hex_unchecked(
                    "0x04718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d",
                ))
                .unwrap(),
            ),
            parent_fee_token_address: ContractAddress(
                PatriciaKey::try_from(Felt::from_hex_unchecked(
                    "0x49d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7",
                ))
                .unwrap(),
            ),
            versioned_constants: ChainVersionedConstants::default(),

            eth_core_contract_address: eth_core_contract_address::MAINNET.parse().expect("parsing a constant"),

            eth_gps_statement_verifier: eth_gps_statement_verifier::MAINNET.parse().expect("parsing a constant"),

            latest_protocol_version: StarknetVersion::LATEST,
            block_time: Duration::from_secs(30),

            no_empty_blocks: false,

            bouncer_config: BouncerConfig::default(),

            // We are not producing blocks for these chains.
            sequencer_address: ContractAddress(
                PatriciaKey::try_from(Felt::from_hex_unchecked(
                    "0x1176a1bd84444c89232ec27754698e5d2e7e1a7f1539f12027f28b23ec9f3d8",
                ))
                .unwrap(),
            ),

            private_key: Some(ZeroingPrivateKey::default()),

            mempool_mode: MempoolMode::Timestamp,
            mempool_max_transactions: 10_000,
            mempool_max_declare_transactions: Some(20),
            mempool_ttl: Some(Duration::from_secs(60 * 60)), // an hour?
            mempool_min_tip_bump: 0.1,
            l2_gas_price: L2GasPrice::starknet_mainnet(),

            block_production_concurrency: BlockProductionConfig::default(),

            l1_messages_replay_max_duration: default_l1_messages_replay_max_duration(),
            l1_messages_finality_blocks: default_l1_messages_finality_blocks(),
        }
    }

    pub fn starknet_sepolia() -> Self {
        Self {
            chain_name: "Starknet Sepolia".into(),
            chain_id: ChainId::Sepolia,
            feeder_gateway_url: Url::parse("https://feeder.alpha-sepolia.starknet.io/feeder_gateway/").unwrap(),
            gateway_url: Url::parse("https://alpha-sepolia.starknet.io/gateway/").unwrap(),
            eth_core_contract_address: eth_core_contract_address::SEPOLIA_TESTNET.parse().expect("parsing a constant"),
            eth_gps_statement_verifier: eth_gps_statement_verifier::SEPOLIA_TESTNET
                .parse()
                .expect("parsing a constant"),
            ..Self::starknet_mainnet()
        }
    }

    pub fn starknet_integration() -> Self {
        Self {
            chain_name: "Starknet Sepolia Integration".into(),
            chain_id: ChainId::IntegrationSepolia,
            feeder_gateway_url: Url::parse("https://feeder.integration-sepolia.starknet.io/feeder_gateway/").unwrap(),
            gateway_url: Url::parse("https://integration-sepolia.starknet.io/gateway/").unwrap(),
            eth_core_contract_address: eth_core_contract_address::SEPOLIA_INTEGRATION
                .parse()
                .expect("parsing a constant"),
            eth_gps_statement_verifier: eth_gps_statement_verifier::SEPOLIA_INTEGRATION
                .parse()
                .expect("parsing a constant"),
            ..Self::starknet_mainnet()
        }
    }

    pub fn madara_devnet() -> Self {
        Self {
            chain_name: "Madara".into(),
            chain_id: ChainId::Other("MADARA_DEVNET".into()),
            feeder_gateway_url: Url::parse("http://localhost:8080/feeder_gateway/").unwrap(),
            gateway_url: Url::parse("http://localhost:8080/gateway/").unwrap(),
            sequencer_address: Felt::from_hex_unchecked("0x123").try_into().unwrap(),
            ..ChainConfig::starknet_sepolia()
        }
    }

    pub fn madara_test() -> Self {
        Self {
            chain_name: "Test".into(),
            chain_id: ChainId::Other("MADARA_TEST".into()),
            feeder_gateway_url: Url::parse("http://localhost:8080/feeder_gateway/").unwrap(),
            gateway_url: Url::parse("http://localhost:8080/gateway/").unwrap(),
            // A random sequencer address for fee transfers to work in block production.
            sequencer_address: Felt::from_hex_unchecked(
                "0x211b748338b39fe8fa353819d457681aa50ac598a3db84cacdd6ece0a17e1f3",
            )
            .try_into()
            .unwrap(),
            // Disable finality for fast test execution
            l1_messages_finality_blocks: 0,
            ..ChainConfig::starknet_sepolia()
        }
    }

    pub fn exec_constants_by_protocol_version(
        &self,
        version: StarknetVersion,
    ) -> Result<VersionedConstants, UnsupportedProtocolVersion> {
        for (k, constants) in self.versioned_constants.0.iter().rev() {
            if k <= &version {
                return Ok(constants.clone());
            }
        }
        Err(UnsupportedProtocolVersion(version))
    }

    pub fn blockifier_chain_info(&self) -> ChainInfo {
        ChainInfo {
            chain_id: self.chain_id.clone(),
            fee_token_addresses: FeeTokenAddresses {
                strk_fee_token_address: self.native_fee_token_address,
                eth_fee_token_address: self.parent_fee_token_address,
            },
            // Is l3 for blockifier means L1 addresses are starknet addresses.
            is_l3: self.settlement_chain_kind == SettlementChainKind::Starknet,
        }
    }
}

// TODO: the motivation for these doc comments is to move them into a proper app chain developer documentation, with a
// proper page about tuning the block production performance.
/// BTreeMap ensures order.
#[derive(Debug, Clone)]
pub struct ChainVersionedConstants(pub BTreeMap<StarknetVersion, VersionedConstants>);

impl<'de> Deserialize<'de> for ChainVersionedConstants {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct ChainVersionedConstantsVisitor;

        impl<'de> Visitor<'de> for ChainVersionedConstantsVisitor {
            type Value = ChainVersionedConstants;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a map of StarknetVersion to VersionedConstants")
            }

            fn visit_map<M>(self, mut access: M) -> Result<Self::Value, M::Error>
            where
                M: MapAccess<'de>,
            {
                let mut map = BTreeMap::new();
                while let Some((key, value)) = access.next_entry::<String, RawVersionedConstants>()? {
                    map.insert(key.parse().map_err(serde::de::Error::custom)?, value.into());
                }
                Ok(ChainVersionedConstants(map))
            }
        }

        deserializer.deserialize_map(ChainVersionedConstantsVisitor)
    }
}

impl<const N: usize> From<[(StarknetVersion, VersionedConstants); N]> for ChainVersionedConstants {
    fn from(arr: [(StarknetVersion, VersionedConstants); N]) -> Self {
        ChainVersionedConstants(arr.into())
    }
}

impl Default for ChainVersionedConstants {
    fn default() -> Self {
        use blockifier::blockifier_versioned_constants::*;
        [
            (StarknetVersion::V0_13_0, VERSIONED_CONSTANTS_V0_13_0.deref().clone()),
            (StarknetVersion::V0_13_1, VERSIONED_CONSTANTS_V0_13_1.deref().clone()),
            (StarknetVersion::V0_13_1_1, VERSIONED_CONSTANTS_V0_13_1_1.deref().clone()),
            (StarknetVersion::V0_13_2, VERSIONED_CONSTANTS_V0_13_2.deref().clone()),
            (StarknetVersion::V0_13_2_1, VERSIONED_CONSTANTS_V0_13_2_1.deref().clone()),
            (StarknetVersion::V0_13_3, VERSIONED_CONSTANTS_V0_13_3.deref().clone()),
            (StarknetVersion::V0_13_4, VERSIONED_CONSTANTS_V0_13_4.deref().clone()),
            (StarknetVersion::V0_13_5, VERSIONED_CONSTANTS_V0_13_5.deref().clone()),
            (StarknetVersion::V0_14_0, VERSIONED_CONSTANTS_V0_14_0.deref().clone()),
        ]
        .into()
    }
}

impl ChainVersionedConstants {
    pub fn add(&mut self, version: StarknetVersion, constants: VersionedConstants) {
        self.0.insert(version, constants);
    }

    pub fn merge(&mut self, other: Self) {
        self.0.extend(other.0);
    }

    pub fn from_file(version_with_path: BTreeMap<String, String>) -> Result<Self> {
        let mut result = BTreeMap::new();

        for (version, path) in version_with_path {
            // Change the current directory to Madara root
            let mut file = File::open(Path::new(&path)).with_context(|| format!("Failed to open file: {}", path))?;

            let mut contents = String::new();
            file.read_to_string(&mut contents).with_context(|| format!("Failed to read contents of file: {}", path))?;

            let constants: RawVersionedConstants =
                serde_json::from_str(&contents).with_context(|| format!("Failed to parse JSON in file: {}", path))?;

            let parsed_version =
                version.parse().with_context(|| format!("Failed to parse version string: {}", version))?;

            result.insert(parsed_version, constants.into());
        }

        Ok(ChainVersionedConstants(result))
    }
}

pub fn deserialize_starknet_version<'de, D>(deserializer: D) -> Result<StarknetVersion, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    StarknetVersion::from_str(&s).map_err(serde::de::Error::custom)
}

pub fn serialize_starknet_version<S>(version: &StarknetVersion, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    version.to_string().serialize(serializer)
}

pub fn serialize_duration<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    if duration.subsec_nanos() == 0 {
        format!("{}s", duration.as_secs()).serialize(serializer)
    } else {
        format!("{}ms", duration.as_millis()).serialize(serializer)
    }
}

pub fn serialize_optional_duration<S>(duration: &Option<Duration>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match duration {
        Some(d) => serialize_duration(d, serializer),
        None => serializer.serialize_none(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use blockifier::blockifier_versioned_constants::ResourceCost;
    use rstest::*;
    use serde_json::Value;
    use starknet_types_core::felt::Felt;

    #[rstest]
    fn test_mainnet_from_yaml() {
        // Change the current directory
        std::env::set_current_dir("../../../../").expect("Failed to change directory");
        let chain_config: ChainConfig =
            ChainConfig::from_yaml(Path::new("configs/presets/mainnet.yaml")).expect("failed to get cfg");

        assert_eq!(chain_config.chain_name, "Starknet Mainnet");
        assert_eq!(chain_config.chain_id, ChainId::Mainnet);

        let native_fee_token_address =
            Felt::from_hex("0x04718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d").unwrap();
        let parent_fee_token_address =
            Felt::from_hex("0x049d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7").unwrap();
        assert_eq!(chain_config.native_fee_token_address, ContractAddress::try_from(native_fee_token_address).unwrap());
        assert_eq!(chain_config.parent_fee_token_address, ContractAddress::try_from(parent_fee_token_address).unwrap());

        // Check versioned constants
        // Load and parse the JSON file
        let json_content =
            fs::read_to_string("madara/crates/primitives/chain_config/resources/versioned_constants_13_0.json")
                .expect("Failed to read JSON file");
        let json: Value = serde_json::from_str(&json_content).expect("Failed to parse JSON");

        // Get the VersionedConstants for version 0.13.0
        let constants = chain_config.versioned_constants.0.get(&StarknetVersion::from_str("0.13.0").unwrap()).unwrap();

        // Check top-level fields
        assert_eq!(constants.invoke_tx_max_n_steps, json["invoke_tx_max_n_steps"].as_u64().unwrap() as u32);
        assert_eq!(constants.max_recursion_depth, json["max_recursion_depth"].as_u64().unwrap() as usize);
        assert_eq!(constants.validate_max_n_steps, json["validate_max_n_steps"].as_u64().unwrap() as u32);
        assert_eq!(constants.segment_arena_cells, json["segment_arena_cells"].as_bool().unwrap());

        // Check L2ResourceGasCosts
        let l2_costs = &constants.deprecated_l2_resource_gas_costs;
        assert_eq!(l2_costs.gas_per_data_felt, ResourceCost::from_integer(0));
        assert_eq!(l2_costs.event_key_factor, ResourceCost::from_integer(0));
        assert_eq!(l2_costs.gas_per_code_byte, ResourceCost::from_integer(0));

        assert_eq!(chain_config.latest_protocol_version, StarknetVersion::from_str("0.13.2").unwrap());
        assert_eq!(chain_config.block_time, Duration::from_secs(30));

        assert_eq!(
            chain_config.sequencer_address,
            ContractAddress::try_from(
                Felt::from_str("0x1176a1bd84444c89232ec27754698e5d2e7e1a7f1539f12027f28b23ec9f3d8").unwrap()
            )
            .unwrap()
        );
        assert_eq!(chain_config.eth_core_contract_address, "0xc662c410C0ECf747543f5bA90660f6ABeBD9C8c4");
    }

    #[rstest]
    fn test_exec_constants() {
        let chain_config = ChainConfig {
            config_version: CURRENT_CONFIG_VERSION,
            versioned_constants: [
                (StarknetVersion::new(0, 1, 5, 0), {
                    let mut constants = VersionedConstants::default();
                    constants.validate_max_n_steps = 5;
                    constants
                }),
                (StarknetVersion::new(0, 2, 0, 0), {
                    let mut constants = VersionedConstants::default();
                    constants.validate_max_n_steps = 10;
                    constants
                }),
            ]
            .into(),
            ..ChainConfig::madara_test()
        };

        assert_eq!(
            chain_config
                .exec_constants_by_protocol_version(StarknetVersion::new(0, 1, 5, 0))
                .unwrap()
                .validate_max_n_steps,
            5
        );
        assert_eq!(
            chain_config
                .exec_constants_by_protocol_version(StarknetVersion::new(0, 1, 6, 0))
                .unwrap()
                .validate_max_n_steps,
            5
        );
        assert_eq!(
            chain_config
                .exec_constants_by_protocol_version(StarknetVersion::new(0, 1, 7, 0))
                .unwrap()
                .validate_max_n_steps,
            5
        );
        assert_eq!(
            chain_config
                .exec_constants_by_protocol_version(StarknetVersion::new(0, 2, 0, 0))
                .unwrap()
                .validate_max_n_steps,
            10
        );
        assert_eq!(
            chain_config
                .exec_constants_by_protocol_version(StarknetVersion::new(0, 2, 5, 0))
                .unwrap()
                .validate_max_n_steps,
            10
        );
        assert_eq!(
            chain_config
                .exec_constants_by_protocol_version(StarknetVersion::new(1, 0, 0, 0))
                .unwrap()
                .validate_max_n_steps,
            10
        );
        assert!(chain_config.exec_constants_by_protocol_version(StarknetVersion::new(0, 0, 0, 0)).is_err(),);
    }
}
