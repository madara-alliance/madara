use std::{collections::HashMap, time::SystemTime};

use dc_block_import::{UnverifiedFullBlock, UnverifiedHeader};
use dp_block::header::GasPrices;
use dp_chain_config::ChainConfig;
use dp_convert::ToFelt;
use dp_state_update::{ContractStorageDiffItem, StateDiff, StorageEntry};
use starknet_api::{core::ContractAddress, state::StorageKey};
use starknet_types_core::felt::Felt;

use self::{
    balances::InitialBalances,
    classes::{InitiallyDeclaredClass, InitiallyDeclaredClasses},
    contracts::InitiallyDeployedContracts,
};

mod balances;
mod classes;
mod contracts;

#[derive(Debug, Clone, Default)]
pub struct StorageDiffs(HashMap<ContractAddress, HashMap<StorageKey, Felt>>);
impl StorageDiffs {
    pub fn contract_mut(&mut self, contract_address: ContractAddress) -> &mut HashMap<StorageKey, Felt> {
        self.0.entry(contract_address).or_default()
    }

    pub fn as_state_diff(&self) -> Vec<ContractStorageDiffItem> {
        self.0
            .iter()
            .map(|(contract, map)| ContractStorageDiffItem {
                address: contract.to_felt(),
                storage_entries: map.iter().map(|(key, &value)| StorageEntry { key: key.to_felt(), value }).collect(),
            })
            .collect()
    }
}

// We allow ourselves to lie about the contract_address. This is because we want the UDC and the two ERC20 contracts to have well known addresses on every chain.
// TODO: remove these class hashes, they should be computed - we don't need to lie about those, we don't care what their hash ends up being.

/// Universal Deployer Contract.
const UDC_CLASS_DEFINITION: &[u8] =
    include_bytes!("../../../../../cairo/target/dev/madara_contracts_UniversalDeployer.contract_class.json");
const UDC_CLASS_HASH: Felt =
    Felt::from_hex_unchecked("0x07b3e05f48f0c69e4a65ce5e076a66271a527aff2c34ce1083ec6e1526997a69");
const UDC_CONTRACT_ADDRESS: Felt =
    Felt::from_hex_unchecked("0x041a78e741e5af2fec34b695679bc6891742439f7afb8484ecd7766661ad02bf");

const ERC20_CLASS_DEFINITION: &[u8] =
    include_bytes!("../../../../../cairo/target/dev/madara_contracts_ERC20.contract_class.json");
const ERC20_CLASS_HASH: Felt =
    Felt::from_hex_unchecked("0x04ad3c1dc8413453db314497945b6903e1c766495a1e60492d44da9c2a986e4b");
const ERC20_STRK_CONTRACT_ADDRESS: Felt =
    Felt::from_hex_unchecked("0x04718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d");
const ERC20_ETH_CONTRACT_ADDRESS: Felt =
    Felt::from_hex_unchecked("0x049d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7");

const ACCOUNT_CLASS_DEFINITION: &[u8] =
    include_bytes!("../../../../../cairo/target/dev/madara_contracts_AccountUpgradeable.contract_class.json");
const ACCOUNT_CLASS_HASH: Felt = Felt::from_hex_unchecked("0xFFFFFFAFAFAFAFAFAFA9b9b9b");

/// High level description of the genesis block.
#[derive(Clone, Debug, Default)]
pub struct ChainGenesisDescription {
    pub initial_balances: InitialBalances,
    pub declared_classes: InitiallyDeclaredClasses,
    pub deployed_contracts: InitiallyDeployedContracts,
}

impl ChainGenesisDescription {
    pub fn base_config(self) -> Self {
        Self {
            initial_balances: InitialBalances::default(),
            declared_classes: InitiallyDeclaredClasses::default()
                .with(InitiallyDeclaredClass::new_legacy(UDC_CLASS_HASH, UDC_CLASS_DEFINITION))
                .with(InitiallyDeclaredClass::new_sierra(ERC20_CLASS_HASH, Felt::ONE, ERC20_CLASS_DEFINITION)),
            deployed_contracts: InitiallyDeployedContracts::default()
                .with(UDC_CONTRACT_ADDRESS, UDC_CLASS_HASH)
                .with(ERC20_ETH_CONTRACT_ADDRESS, ERC20_CLASS_HASH)
                .with(ERC20_STRK_CONTRACT_ADDRESS, ERC20_CLASS_HASH),
        }
    }

    pub fn build(self, chain_config: &ChainConfig) -> anyhow::Result<UnverifiedFullBlock> {
        let mut storage_diffs = Default::default();
        self.initial_balances.to_storage_diffs(&chain_config, &mut storage_diffs);

        Ok(UnverifiedFullBlock {
            header: UnverifiedHeader {
                parent_block_hash: None,
                sequencer_address: chain_config.sequencer_address.to_felt(),
                block_timestamp: SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .expect("Current time is before unix epoch!")
                    .as_secs(),
                protocol_version: chain_config.latest_protocol_version,
                l1_gas_price: GasPrices::default(),
                l1_da_mode: dp_block::header::L1DataAvailabilityMode::Blob,
            },
            state_diff: StateDiff {
                storage_diffs: storage_diffs.as_state_diff(),
                deprecated_declared_classes: self.declared_classes.as_legacy_state_diff(),
                declared_classes: self.declared_classes.as_state_diff(),
                deployed_contracts: self.deployed_contracts.as_state_diff(),
                replaced_classes: vec![],
                nonces: vec![],
            },
            declared_classes: self.declared_classes.into_loaded_classes()?,
            ..Default::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const CLASS_DUMMY_ACCOUNT: &[u8] = include_bytes!("../../resources/dummy_account.json");

    #[rstest]
    fn test_make_chain() {
        // let contract_1

        make_genesis_block(ChainConfig::test_config(), InitialBalances::with(self, contract_address, bal))
    }
}
