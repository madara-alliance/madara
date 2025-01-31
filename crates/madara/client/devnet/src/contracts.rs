use mp_state_update::DeployedContractItem;
use starknet_types_core::felt::Felt;
use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
pub struct InitiallyDeployedContracts(HashMap<Felt, Felt>);

impl InitiallyDeployedContracts {
    pub fn with(mut self, address: Felt, class_hash: Felt) -> Self {
        self.insert(address, class_hash);
        self
    }

    pub fn insert(&mut self, address: Felt, class_hash: Felt) {
        self.0.insert(address, class_hash);
    }

    pub fn as_state_diff(&self) -> Vec<DeployedContractItem> {
        self.0.iter().map(|(&address, &class_hash)| DeployedContractItem { address, class_hash }).collect()
    }
}
