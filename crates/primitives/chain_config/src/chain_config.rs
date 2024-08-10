use crate::StarknetVersion;
use blockifier::{
    bouncer::{BouncerConfig, BouncerWeights, BuiltinCount},
    versioned_constants::VersionedConstants,
};
use primitive_types::H160;
use starknet_api::core::{ChainId, ContractAddress, PatriciaKey};
use starknet_core::types::Felt;
use std::{collections::BTreeMap, ops::Deref, time::Duration};

pub mod eth_core_contract_address {
    pub const MAINNET: &str = "0xc662c410C0ECf747543f5bA90660f6ABeBD9C8c4";
    pub const SEPOLIA_TESTNET: &str = "0xE2Bb56ee936fd6433DC0F6e7e3b8365C906AA057";
    pub const SEPOLIA_INTEGRATION: &str = "0x4737c0c1B4D5b1A687B42610DdabEE781152359c";
}

const BLOCKIFIER_VERSIONED_CONSTANTS_JSON_0_13_0: &[u8] = include_bytes!("../resources/versioned_constants_13_0.json");
const BLOCKIFIER_VERSIONED_CONSTANTS_JSON_0_13_1: &[u8] = include_bytes!("../resources/versioned_constants_13_1.json");
const BLOCKIFIER_VERSIONED_CONSTANTS_JSON_0_13_1_1: &[u8] =
    include_bytes!("../resources/versioned_constants_13_1_1.json");

lazy_static::lazy_static! {
    pub static ref BLOCKIFIER_VERSIONED_CONSTANTS_0_13_1_1: VersionedConstants =
        serde_json::from_slice(BLOCKIFIER_VERSIONED_CONSTANTS_JSON_0_13_1_1).unwrap();
    pub static ref BLOCKIFIER_VERSIONED_CONSTANTS_0_13_1: VersionedConstants =
        serde_json::from_slice(BLOCKIFIER_VERSIONED_CONSTANTS_JSON_0_13_1).unwrap();
    pub static ref BLOCKIFIER_VERSIONED_CONSTANTS_0_13_0: VersionedConstants =
        serde_json::from_slice(BLOCKIFIER_VERSIONED_CONSTANTS_JSON_0_13_0).unwrap();
}

#[derive(Debug)]
pub struct ChainConfig {
    /// Internal chain name.
    pub chain_name: String,
    pub chain_id: ChainId,

    /// For starknet, this is the STRK ERC-20 contract on starknet.
    pub native_fee_token_address: ContractAddress,
    /// For starknet, this is the ETH ERC-20 contract on starknet.
    pub parent_fee_token_address: ContractAddress,

    /// BTreeMap ensures order.
    pub versioned_constants: BTreeMap<StarknetVersion, VersionedConstants>,
    pub latest_protocol_version: StarknetVersion,

    /// Only used for block production.
    pub block_time: Duration,
    /// Only used for block production.
    /// Block time is divided into "ticks": everytime this duration elapses, the pending block is updated.  
    pub pending_block_update_time: Duration,

    /// The bouncer is in charge of limiting block sizes. This is where the max number of step per block, gas etc are.
    /// Only used for block production.
    pub bouncer_config: BouncerConfig,

    /// Only used for block production.
    pub sequencer_address: ContractAddress,

    /// Only used when mempool is enabled.
    /// When deploying an account and invoking a contract at the same time, we want to skip the validation step for the invoke tx.
    /// This number is the maximum nonce the invoke tx can have to qualify for the validation skip.
    pub max_nonce_for_validation_skip: u64,

    /// The Starknet core contract address for the L1 watcher.
    pub eth_core_contract_address: H160,
}

#[derive(thiserror::Error, Debug)]
#[error("Unsupported protocol version: {0}")]
pub struct UnsupportedProtocolVersion(StarknetVersion);

impl ChainConfig {
    /// This is the number of pending ticks (see [`ChainConfig::pending_block_update_time`]) in a block.
    pub fn n_pending_ticks_per_block(&self) -> usize {
        (self.block_time.as_millis() / self.pending_block_update_time.as_millis()) as usize
    }

    pub fn exec_constants_by_protocol_version(
        &self,
        version: StarknetVersion,
    ) -> Result<VersionedConstants, UnsupportedProtocolVersion> {
        for (k, constants) in self.versioned_constants.iter().rev() {
            if k <= &version {
                return Ok(constants.clone());
            }
        }
        Err(UnsupportedProtocolVersion(version))
    }

    pub fn starknet_mainnet() -> Self {
        // Sources:
        // - https://docs.starknet.io/tools/important-addresses
        // - https://docs.starknet.io/tools/limits-and-triggers (bouncer & block times)
        // - state_diff_size is the blob size limit of ethereum
        // - pending_block_update_time: educated guess
        // - bouncer builtin_count, message_segment_length, n_events, state_diff_size are probably wrong

        Self {
            chain_name: "Starknet Mainnet".into(),
            chain_id: ChainId::Mainnet,
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
            versioned_constants: [
                (StarknetVersion::STARKNET_VERSION_0_13_0, BLOCKIFIER_VERSIONED_CONSTANTS_0_13_0.deref().clone()),
                (StarknetVersion::STARKNET_VERSION_0_13_1, BLOCKIFIER_VERSIONED_CONSTANTS_0_13_1.deref().clone()),
                (StarknetVersion::STARKNET_VERSION_0_13_1_1, BLOCKIFIER_VERSIONED_CONSTANTS_0_13_1_1.deref().clone()),
                (StarknetVersion::STARKNET_VERSION_0_13_2, VersionedConstants::latest_constants().clone()),
            ]
            .into(),

            eth_core_contract_address: eth_core_contract_address::MAINNET.parse().expect("parsing a constant"),

            latest_protocol_version: StarknetVersion::STARKNET_VERSION_0_13_2,
            block_time: Duration::from_secs(6 * 60),
            pending_block_update_time: Duration::from_secs(2),

            bouncer_config: BouncerConfig {
                block_max_capacity: BouncerWeights {
                    builtin_count: BuiltinCount {
                        add_mod: usize::MAX,
                        bitwise: usize::MAX,
                        ecdsa: usize::MAX,
                        ec_op: usize::MAX,
                        keccak: usize::MAX,
                        mul_mod: usize::MAX,
                        pedersen: usize::MAX,
                        poseidon: usize::MAX,
                        range_check: usize::MAX,
                        range_check96: usize::MAX,
                    },
                    gas: 5_000_000,
                    n_steps: 40_000_000,
                    message_segment_length: usize::MAX,
                    n_events: usize::MAX,
                    state_diff_size: 131072,
                },
            },
            // We are not producing blocks for these chains.
            sequencer_address: ContractAddress::default(),
            max_nonce_for_validation_skip: 2,
        }
    }

    pub fn starknet_sepolia() -> Self {
        Self {
            chain_name: "Starknet Sepolia".into(),
            chain_id: ChainId::Sepolia,
            eth_core_contract_address: eth_core_contract_address::SEPOLIA_TESTNET.parse().expect("parsing a constant"),
            ..Self::starknet_mainnet()
        }
    }

    pub fn starknet_integration() -> Self {
        Self {
            chain_name: "Starknet Sepolia Integration".into(),
            chain_id: ChainId::IntegrationSepolia,
            eth_core_contract_address: eth_core_contract_address::SEPOLIA_INTEGRATION
                .parse()
                .expect("parsing a constant"),
            ..Self::starknet_mainnet()
        }
    }

    pub fn test_config() -> Self {
        Self {
            chain_name: "Test".into(),
            chain_id: ChainId::Other("MADARA_TEST".into()),
            ..ChainConfig::starknet_sepolia()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exec_constants() {
        let chain_config = ChainConfig {
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
            ..ChainConfig::test_config()
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
