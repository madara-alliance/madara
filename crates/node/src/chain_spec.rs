use deoxys_runtime::{AuraConfig, GrandpaConfig, RuntimeGenesisConfig, SealingMode, SystemConfig, WASM_BINARY};
use pallet_starknet::genesis_loader::GenesisData;
use pallet_starknet::GenesisConfig;
use sc_service::ChainType;
use serde::{Deserialize, Serialize};
use sp_consensus_aura::sr25519::AuthorityId as AuraId;
use sp_consensus_grandpa::AuthorityId as GrandpaId;
use sp_core::storage::Storage;
use sp_core::{Pair, Public};
use sp_state_machine::BasicExternalities;
use starknet_providers::sequencer::models::BlockId;
use starknet_providers::SequencerGatewayProvider;
use tokio::runtime::Runtime;

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
/// can then use this information to adjust accordingly.
impl sp_runtime::BuildStorage for DevGenesisExt {
    fn assimilate_storage(&self, storage: &mut Storage) -> Result<(), String> {
        BasicExternalities::execute_with_storage(storage, || {
            deoxys_runtime::Sealing::set(&self.sealing);
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

pub fn deoxys_config(sealing: SealingMode, chain_id: &str) -> Result<DevChainSpec, String> {
    let wasm_binary = WASM_BINARY.ok_or_else(|| "Development wasm not available".to_string())?;
    let genesis_loader = load_genesis_state()?;

    Ok(DevChainSpec::from_genesis(
        // Name
        "Starknet",
        // Chain ID
        chain_id,
        // Chain Type
        ChainType::Live,
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

#[allow(deprecated)]
fn load_genesis_state() -> Result<GenesisData, String> {
    log::info!("🧪 Fetching genesis block");
    let runtime = Runtime::new().unwrap();
    let provider = SequencerGatewayProvider::starknet_alpha_mainnet();
    let diff = runtime.block_on(async {
        provider
            .get_state_update(BlockId::Number(0))
            .await
            .map(|state_update| state_update.state_diff)
            .map_err(|e| format!("Failed to get state update {e}"))
    })?;

    Ok(GenesisData::from(diff))
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
        // Starknet Genesis configuration.
        starknet: starknet_genesis_config,
    }
}
