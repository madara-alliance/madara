use std::collections::HashMap;

use anyhow::Context;
use mp_chain_config::ChainConfig;
use starknet_api::core::ContractAddress;
use starknet_types_core::felt::Felt;

use super::StorageDiffs;

#[derive(Clone, Debug, Default)]
pub struct ContractFeeTokensBalance {
    pub fri: Felt,
    pub wei: Felt,
}

impl ContractFeeTokensBalance {
    pub fn as_u128_fri_wei(&self) -> anyhow::Result<(u128, u128)> {
        let fri =
            self.fri.try_into().with_context(|| format!("Converting STRK balance felt {:#x} to u128", self.fri))?;
        let wei =
            self.wei.try_into().with_context(|| format!("Converting ETH balance felt {:#x} to u128", self.wei))?;
        Ok((fri, wei))
    }
}

#[derive(Clone, Debug, Default)]
pub struct InitialBalances(pub HashMap<ContractAddress, ContractFeeTokensBalance>);

impl InitialBalances {
    #[tracing::instrument(skip(self, contract_address, bal), fields(module = "InitialBalances"))]
    pub fn with(mut self, contract_address: ContractAddress, bal: ContractFeeTokensBalance) -> Self {
        self.insert(contract_address, bal);
        self
    }

    #[tracing::instrument(skip(self, contract_address, bal), fields(module = "InitialBalances"))]
    pub fn insert(&mut self, contract_address: ContractAddress, bal: ContractFeeTokensBalance) {
        self.0.insert(contract_address, bal);
    }

    #[tracing::instrument(skip(self, chain_config, storage_diffs), fields(module = "InitialBalances"))]
    pub fn to_storage_diffs(&self, chain_config: &ChainConfig, storage_diffs: &mut StorageDiffs) {
        for (contract_address, bal) in &self.0 {
            // Storage key where the balance of that contract is stored. For both STRK and ETH it ends up
            // being the same key.

            // The balance is a U256, following the ethereum erc20 spec. This does not fit into a felt (252 bit), so the cairo compiler
            // ends up splitting the balance in two.
            // For now we never use high - blockifier does not entirely support it, as the total supply of STRK/ETH would not reach the high bits.
            // TODO: check this is true ^

            let low_key = starknet_api::abi::abi_utils::get_fee_token_var_address(*contract_address);
            // let high_key = blockifier::abi::sierra_types::next_storage_key(&low_key)?;

            // ETH erc20
            let erc20_contract = chain_config.parent_fee_token_address;
            let kv = storage_diffs.contract_mut(erc20_contract);
            kv.insert(low_key, bal.wei);
            // kv.insert(high_key, Felt::ZERO);

            // STRK erc20
            let erc20_contract = chain_config.native_fee_token_address;
            let kv = storage_diffs.contract_mut(erc20_contract);
            kv.insert(low_key, bal.fri);
            // kv.insert(high_key, Felt::ZERO);
        }
    }
}
