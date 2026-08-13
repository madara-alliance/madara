use once_cell::sync::Lazy;
use serde::Deserialize;
use starknet_types_core::felt::Felt;
use std::collections::{BTreeMap, HashSet};

use crate::core::storage::function_selector;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupportedContract {
    pub name: String,
    pub class_hash: Felt,
    pub supported_functions: Vec<SupportedFunction>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupportedFunction {
    pub name: String,
    pub selector: Felt,
}

#[derive(Debug, Deserialize)]
struct SupportedContractJson {
    class_hash: String,
    supported_functions: Vec<String>,
}

static SUPPORTED_CONTRACTS_TYPED: Lazy<Vec<SupportedContract>> = Lazy::new(|| {
    let raw: BTreeMap<String, SupportedContractJson> =
        serde_json::from_str(crate::SUPPORTED_CONTRACTS).expect("supported_contracts.json must be valid JSON");

    raw.into_iter()
        .map(|(name, contract)| {
            let class_hash = Felt::from_hex(&contract.class_hash)
                .unwrap_or_else(|_| panic!("supported_contracts.json has invalid class_hash for {name}"));
            let supported_functions = contract
                .supported_functions
                .into_iter()
                .map(|name| SupportedFunction { selector: function_selector(&name), name })
                .collect();
            SupportedContract { name, class_hash, supported_functions }
        })
        .collect()
});

pub fn supported_contracts() -> &'static [SupportedContract] {
    &SUPPORTED_CONTRACTS_TYPED
}

pub(crate) fn supported_contract(name: &str) -> Option<&'static SupportedContract> {
    supported_contracts().iter().find(|contract| contract.name == name)
}

pub fn supported_class_hashes() -> HashSet<Felt> {
    supported_contracts().iter().map(|contract| contract.class_hash).collect()
}

pub fn supported_selectors() -> HashSet<Felt> {
    supported_contracts()
        .iter()
        .flat_map(|contract| contract.supported_functions.iter().map(|function| function.selector))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_supported_contracts_manifest() {
        let contracts = supported_contracts();
        assert_eq!(contracts.len(), 3);
        assert!(supported_class_hashes()
            .contains(&Felt::from_hex_unchecked("0x05e9bdfbd0b2b461a42052f43a38663b1d53f7ce8a9537bdc06b857b7508a13a")));
        assert!(supported_class_hashes()
            .contains(&Felt::from_hex_unchecked("0x00049e91ccb24fcf4acec4a24896092d9387a97865dcb0e6f98503399564b452")));
        assert!(supported_class_hashes()
            .contains(&Felt::from_hex_unchecked("0x01243f5fe98954ece09eec4ccc03567b2b3e51bef4fa3f1c4ea2ab8c57e9c3b8")));
    }

    #[test]
    fn derives_selectors_from_supported_function_names() {
        let selectors = supported_selectors();
        assert!(selectors.contains(&function_selector("settle_trade_v3")));
        assert!(selectors.contains(&function_selector("set_prices_and_funding_snapshot")));
        assert!(selectors.contains(&function_selector("transfer")));
        assert!(selectors.contains(&function_selector("transfer_with_comparator_mismatch")));
    }

    #[test]
    fn resolves_contract_local_supported_functions() {
        let contract = supported_contract("RustExecTransfer").expect("devnet transfer contract must be registered");
        assert_eq!(contract.supported_functions.len(), 2);
        assert_eq!(contract.supported_functions[0].name, "transfer");
        assert_eq!(contract.supported_functions[1].name, "transfer_with_comparator_mismatch");
    }
}
