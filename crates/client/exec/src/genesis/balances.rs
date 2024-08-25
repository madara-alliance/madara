use std::collections::HashMap;

use dp_chain_config::ChainConfig;
use starknet_types_core::felt::Felt;
use starknet_api::core::ContractAddress;

use super::StorageDiffs;

#[derive(Clone, Debug, Default)]
pub struct InitialBalance {
    pub fri: Felt,
    pub wei: Felt,
}

#[derive(Clone, Debug, Default)]
pub struct InitialBalances(pub HashMap<ContractAddress, InitialBalance>);

impl InitialBalances {
    pub fn with(mut self, contract_address: ContractAddress, bal: InitialBalance) -> Self {
        self.insert(contract_address, bal);
        self
    }

    pub fn insert(&mut self, contract_address: ContractAddress, bal: InitialBalance) {
        self.0.insert(contract_address, bal);
    }

    pub fn to_storage_diffs(&self, chain_config: &ChainConfig, storage_diffs: &mut StorageDiffs) {
        for (contract_address, bal) in &self.0 {
            // Storage key where the balance of that contract is stored. For both STRK and ETH it ends up
            // being the same key.
            let low_key = blockifier::abi::abi_utils::get_fee_token_var_address(*contract_address);
            // let high_key = blockifier::abi::sierra_types::next_storage_key(&low_key)?;

            // ETH erc20
            let erc20_contract = chain_config.parent_fee_token_address;
            let kv = storage_diffs.contract_mut(erc20_contract);
            kv.insert(low_key, bal.wei.into());
            // kv.insert(high_key, Felt::ZERO);

            // STRK erc20
            let erc20_contract = chain_config.native_fee_token_address;
            let kv = storage_diffs.contract_mut(erc20_contract);
            kv.insert(low_key, bal.fri.into());
            // kv.insert(high_key, Felt::ZERO);
        }
    }
}
