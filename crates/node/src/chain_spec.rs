use std::path::PathBuf;

use madara_runtime::{AuraConfig, GrandpaConfig, RuntimeGenesisConfig, SealingMode, SystemConfig, WASM_BINARY};
use pallet_starknet::genesis_loader::{GenesisData, GenesisLoader, HexFelt};
use pallet_starknet::GenesisConfig;
use sc_service::{BasePath, ChainType};
use serde::{Deserialize, Serialize};
use sp_consensus_aura::sr25519::AuthorityId as AuraId;
use sp_consensus_grandpa::AuthorityId as GrandpaId;
use sp_core::storage::Storage;
use sp_core::{Pair, Public};
use sp_state_machine::BasicExternalities;
use starknet_providers::sequencer::models::BlockId;
use starknet_providers::SequencerGatewayProvider;
use tokio::runtime::Runtime;

use crate::constants::DEV_CHAIN_ID;

pub const GENESIS_ASSETS_DIR: &str = "genesis-assets/";
pub const GENESIS_ASSETS_FILE: &str = "genesis.json";

/// Specialized `ChainSpec`. This is a specialization of the general Substrate ChainSpec type.
pub type ChainSpec = sc_service::GenericChainSpec<RuntimeGenesisConfig>;

/// Specialized `ChainSpec` for development.
pub type DevChainSpec = sc_service::GenericChainSpec<DevGenesisExt>;

/// Extension for the dev genesis config to support a custom changes to the genesis state.
#[derive(Serialize, Deserialize)]
pub struct DevGenesisExt {
    /// Genesis config.
    genesis_config: RuntimeGenesisConfig,
    /// The sealing mode being used.
    sealing: SealingMode,
}

/// The `sealing` from the `DevGenesisExt` is passed to the runtime via the storage. The runtime
/// can then use this information to adjust accordingly. This is just a common way to pass
/// information from the chain spec to the runtime.
///
/// NOTE: if `sealing` is `None`, then the runtime will use the default sealing mode.
impl sp_runtime::BuildStorage for DevGenesisExt {
    fn assimilate_storage(&self, storage: &mut Storage) -> Result<(), String> {
        BasicExternalities::execute_with_storage(storage, || {
            madara_runtime::Sealing::set(&self.sealing);
        });
        self.genesis_config.assimilate_storage(storage)
    }
}

/// Generate a crypto pair from seed.
pub fn get_from_seed<TPublic: Public>(seed: &str) -> <TPublic::Pair as Pair>::Public {
    TPublic::Pair::from_string(&format!("//{seed}"), None).expect("static values are valid; qed").public()
}

/// Generate an Aura authority key.
pub fn authority_keys_from_seed(s: &str) -> (AuraId, GrandpaId) {
    (get_from_seed::<AuraId>(s), get_from_seed::<GrandpaId>(s))
}

pub fn development_config(sealing: SealingMode, base_path: BasePath) -> Result<DevChainSpec, String> {
    let wasm_binary = WASM_BINARY.ok_or_else(|| "Development wasm not available".to_string())?;
    let chain_id = DEV_CHAIN_ID;
    let genesis_loader = load_genesis()?;

    Ok(DevChainSpec::from_genesis(
        // Name
        "Development",
        // ID
        chain_id,
        ChainType::Development,
        move || {
            DevGenesisExt {
                genesis_config: testnet_genesis(
                    genesis_loader.clone(),
                    wasm_binary,
                    // Initial PoA authorities
                    vec![authority_keys_from_seed("Alice")],
                    true,
                ),
                sealing: sealing.clone(),
            }
        },
        // Bootnodes
        vec![],
        // Telemetry
        None,
        // Protocol ID
        None,
        None,
        // Properties
        None,
        // Extensions
        None,
    ))
}

pub fn local_testnet_config(base_path: BasePath, chain_id: &str) -> Result<ChainSpec, String> {
    let wasm_binary = WASM_BINARY.ok_or_else(|| "Development wasm not available".to_string())?;

    let owned_chain_id = chain_id.to_owned();
    let genesis_loader = load_genesis()?;

    Ok(ChainSpec::from_genesis(
        // Name
        "Local Testnet",
        // ID
        chain_id,
        ChainType::Local,
        move || {
            testnet_genesis(
                genesis_loader.clone(),
                wasm_binary,
                // Initial PoA authorities
                // Intended to be only 2
                vec![authority_keys_from_seed("Alice"), authority_keys_from_seed("Bob")],
                true,
            )
        },
        // Bootnodes
        vec![],
        // Telemetry
        None,
        // Protocol ID
        None,
        // Properties
        None,
        None,
        // Extensions
        None,
    ))
}

pub fn deoxys_config(sealing: SealingMode, base_path: BasePath, chain_id: &str) -> Result<DevChainSpec, String> {
    let wasm_binary = WASM_BINARY.ok_or_else(|| "Development wasm not available".to_string())?;
    let owned_chain_id = chain_id.to_owned();
    let genesis_loader = load_genesis()?;

    Ok(DevChainSpec::from_genesis(
        // Name
        "Starknet",
        // ID
        chain_id,
        ChainType::Development,
        move || {
            log::info!("🔃 Loading Deoxys config");

            DevGenesisExt {
                genesis_config: testnet_genesis(
                    genesis_loader.clone(),
                    wasm_binary,
                    // Initial PoA authorities
                    vec![authority_keys_from_seed("Alice")],
                    true,
                ),
                sealing: sealing.clone(),
            }
        },
        // Bootnodes
        vec![],
        // Telemetry
        None,
        // Protocol ID
        None,
        None,
        // Properties
        None,
        // Extensions
        None,
    ))
}

fn load_genesis() -> Result<GenesisData, String> {
    log::info!("🧪 Fetching genesis block");
    let runtime = Runtime::new().unwrap();
    let genesis_block = runtime.block_on(async {
        SequencerGatewayProvider::starknet_alpha_mainnet()
            .get_block(BlockId::Number(0))
            .await
            .map_err(|e| format!("failed to get block: {e}"))
    })?;

    Ok(GenesisData::from(genesis_block))
}

/// Configure initial storage state for FRAME modules.
fn testnet_genesis(
    genesis_loader: GenesisData,
    wasm_binary: &[u8],
    initial_authorities: Vec<(AuraId, GrandpaId)>,
    _enable_println: bool,
) -> RuntimeGenesisConfig {
    let starknet_genesis_config = GenesisConfig::from(genesis_loader);

    RuntimeGenesisConfig {
        system: SystemConfig {
            // Add Wasm runtime to storage.
            code: wasm_binary.to_vec(),
            _config: Default::default(),
        },
        // Authority-based consensus protocol used for block production
        aura: AuraConfig { authorities: initial_authorities.iter().map(|x| (x.0.clone())).collect() },
        // Deterministic finality mechanism used for block finalization
        grandpa: GrandpaConfig {
            authorities: initial_authorities.iter().map(|x| (x.1.clone(), 1)).collect(),
            _config: Default::default(),
        },
        /// Starknet Genesis configuration.
        starknet: starknet_genesis_config,
    }
}
