use std::collections::{hash_map, HashMap};

use starknet_api::core::ContractAddress;

/// This is used for quickly checking if the contract has been deployed for the same block it is invoked.
/// When force inserting transaction, it may happen that we run into a duplicate deploy_account transaction. Keep a count for that purpose.
#[derive(Debug, Clone, Default)]
pub struct DeployedContracts(HashMap<ContractAddress, u64>);

impl DeployedContracts {
    pub fn decrement(&mut self, address: ContractAddress) {
        match self.0.entry(address) {
            hash_map::Entry::Occupied(mut entry) => {
                *entry.get_mut() -= 1;
                if entry.get() == &0 {
                    // Count is now 0, we can delete the entry.
                    entry.remove();
                }
            }
            hash_map::Entry::Vacant(_) => unreachable!("invariant violated"),
        }
    }

    pub fn increment(&mut self, address: ContractAddress) {
        *self.0.entry(address).or_insert(0) += 1
    }

    pub fn contains(&self, address: &ContractAddress) -> bool {
        self.0.contains_key(address)
    }

    #[cfg(any(test, feature = "testing"))]
    pub fn count(&self) -> u64 {
        self.0.iter().fold(0, |acc, entry| acc + entry.1)
    }
}
