use dc_sync::commitments::{calculate_state_root, compute_root_keyed};
use dp_state_update::{
    ContractStorageDiffItem, DeployedContractItem, NonceUpdate, ReplacedClassItem, StateDiff, StorageEntry,
};
use indexmap::IndexMap;
use rand::{rngs::StdRng, Rng, SeedableRng};
use starknet_types_core::felt::Felt;
use starknet_types_core::hash::{Pedersen, Poseidon, StarkHash};

use crate::felt::{random_felt, FeltMax};

#[derive(Debug, Default)]
pub struct BlockState {
    pub heigh: Option<u64>,
    pub parent_block_hash: Felt,
    pub contracts: IndexMap<Felt, ContractState>,
    pub declared_classes: IndexMap<Felt, Felt>,
}

impl BlockState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get_contract(&self, contract_address: Felt) -> Option<&ContractState> {
        self.contracts.get(&contract_address)
    }

    pub fn get_mut_contract(&mut self, contract_address: Felt) -> Option<&mut ContractState> {
        self.contracts.get_mut(&contract_address)
    }

    pub fn get_random_contract_address(&self, rng: &mut StdRng) -> Option<Felt> {
        let keys = self.contracts.keys().collect::<Vec<_>>();
        if keys.is_empty() {
            None
        } else {
            let contract_address = keys[rng.gen_range(0..keys.len())];
            Some(*contract_address)
        }
    }

    pub fn add_contract(&mut self, contract_address: Felt, class_hash: Felt) {
        self.contracts.insert(contract_address, ContractState { class_hash, nonce: 0, storage: IndexMap::new() });
    }

    pub fn get_class(&self, class_hash: Felt) -> Option<&Felt> {
        self.declared_classes.get(&class_hash)
    }

    pub fn get_random_class_hash(&self, rng: &mut StdRng) -> Option<Felt> {
        let keys = self.declared_classes.keys().collect::<Vec<_>>();
        if keys.is_empty() {
            None
        } else {
            let class_hash = keys[rng.gen_range(0..keys.len())];
            Some(*class_hash)
        }
    }

    pub fn add_class(&mut self, class_hash: Felt, compiled_class_hash: Felt) {
        self.declared_classes.insert(class_hash, compiled_class_hash);
    }

    pub fn next_state(&mut self, seed: u64) -> StateDiff {
        let mut rng = StdRng::seed_from_u64(seed);
        let mut state_diff = StateDiff::default();
        self.heigh = match self.heigh {
            Some(heigh) => Some(heigh + 1),
            None => Some(0),
        };

        // declare 1 new class
        let new_class: (Felt, Felt) = (random_felt(&mut rng, FeltMax::Max), random_felt(&mut rng, FeltMax::Max));
        self.add_class(new_class.0, new_class.1);

        // replace 1 contract's class if there is any
        if let Some(contract_address) = self.get_random_contract_address(&mut rng) {
            let class_hash = self.get_random_class_hash(&mut rng).unwrap();
            self.contracts.get_mut(&contract_address).unwrap().update_class_hash(new_class.0);
            state_diff.replaced_classes.push(ReplacedClassItem { contract_address, class_hash });
        }

        // deploy 2 new contract
        for _ in 0..2 {
            let address = random_felt(&mut rng, FeltMax::Max);
            let class_hash = self.get_random_class_hash(&mut rng).unwrap();
            self.add_contract(address, class_hash);
            state_diff.deployed_contracts.push(DeployedContractItem { address, class_hash });
        }

        // update 1 contract's nonce
        if let Some(contract_address) = self.get_random_contract_address(&mut rng) {
            let contract = self.contracts.get_mut(&contract_address).unwrap();
            let new_nonce = contract.nonce + rng.gen_range(1..5);
            contract.update_nonce(new_nonce);
            state_diff.nonces.push(NonceUpdate { contract_address, nonce: new_nonce.into() });
        }

        // update contract's storage
        if let Some(contract_address) = self.get_random_contract_address(&mut rng) {
            let contract = self.contracts.get_mut(&contract_address).unwrap();
            let key = random_felt(&mut rng, FeltMax::Max);
            let value = random_felt(&mut rng, FeltMax::Max);
            contract.update_storage(key, value);
            state_diff.storage_diffs.push(ContractStorageDiffItem {
                address: contract_address,
                storage_entries: vec![StorageEntry { key, value }],
            });
        }

        state_diff
    }

    pub fn state_root(&self) -> Felt {
        let contract_leafs = self
            .contracts
            .iter()
            .map(|(contract_address, contract_state)| {
                let storage_root = compute_root_keyed::<Pedersen>(
                    &contract_state.storage.iter().map(|(k, v)| (*k, *v)).collect::<Vec<_>>(),
                );
                (
                    *contract_address,
                    Pedersen::hash(
                        &Pedersen::hash(
                            &Pedersen::hash(&contract_state.class_hash, &storage_root),
                            &contract_state.nonce.into(),
                        ),
                        &Felt::ZERO,
                    ),
                )
            })
            .collect::<Vec<_>>();

        let contract_trie_root = compute_root_keyed::<Pedersen>(&contract_leafs);

        let class_leafs = self
            .declared_classes
            .iter()
            .map(|(class_hash, compiled_class_hash)| {
                (
                    *class_hash,
                    Poseidon::hash(&Felt::from_bytes_be_slice(b"CONTRACT_CLASS_LEAF_V0"), compiled_class_hash),
                )
            })
            .collect::<Vec<_>>();

        let class_trie_root = compute_root_keyed::<Poseidon>(&class_leafs);

        calculate_state_root(contract_trie_root, class_trie_root)
    }
}

#[derive(Debug, Default)]
pub struct ContractState {
    pub class_hash: Felt,
    pub nonce: u64,
    pub storage: IndexMap<Felt, Felt>,
}

impl ContractState {
    pub fn new(class_hash: Felt, nonce: u64) -> Self {
        Self { class_hash, nonce, storage: IndexMap::new() }
    }
    pub fn update_class_hash(&mut self, class_hash: Felt) {
        self.class_hash = class_hash;
    }

    pub fn update_nonce(&mut self, nonce: u64) {
        self.nonce = nonce;
    }

    pub fn get_storage(&self, key: Felt) -> Option<&Felt> {
        self.storage.get(&key)
    }

    pub fn update_storage(&mut self, key: Felt, value: Felt) {
        self.storage.insert(key, value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // test with data of testnet block 0
    #[test]
    fn test_block_stat_root() {
        let mut block_state = BlockState::new();
        block_state.add_contract(
            Felt::from_hex_unchecked("0x43abaa073c768ebf039c0c4f46db9acc39e9ec165690418060a652aab39e7d8"),
            Felt::from_hex_unchecked("0x5c478ee27f2112411f86f207605b2e2c58cdb647bac0df27f660ef2252359c6"),
        );
        block_state.add_contract(
            Felt::from_hex_unchecked("0x49d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7"),
            Felt::from_hex_unchecked("0xd0e183745e9dae3e4e78a8ffedcce0903fc4900beace4e0abf192d4c202da3"),
        );
        block_state.add_contract(
            Felt::from_hex_unchecked("0x4c5772d1914fe6ce891b64eb35bf3522aeae1315647314aac58b01137607f3f"),
            Felt::from_hex_unchecked("0xd0e183745e9dae3e4e78a8ffedcce0903fc4900beace4e0abf192d4c202da3"),
        );

        let c1 = block_state
            .get_mut_contract(Felt::from_hex_unchecked(
                "0x4c5772d1914fe6ce891b64eb35bf3522aeae1315647314aac58b01137607f3f",
            ))
            .unwrap();
        c1.update_storage(
            Felt::from_hex_unchecked("0xe8fc4f1b6b3dc661208f9a8a5017a6c059098327e31518722e0a5c3a5a7e86"),
            Felt::from_hex_unchecked("0x1"),
        );
        c1.update_storage(
            Felt::from_hex_unchecked("0x6d56a3a16e0bf05482515c10fcf552437cdd1b7b409a6e8cec88c4b8fa03c13"),
            Felt::from_hex_unchecked("0x1"),
        );

        let c2 = block_state
            .get_mut_contract(Felt::from_hex_unchecked(
                "0x43abaa073c768ebf039c0c4f46db9acc39e9ec165690418060a652aab39e7d8",
            ))
            .unwrap();
        c2.update_storage(
            Felt::from_hex_unchecked("0x3b28019ccfdbd30ffc65951d94bb85c9e2b8434111a000b5afd533ce65f57a4"),
            Felt::from_hex_unchecked("0x12c4df40394d06f157edec8d0e64db61fe0c271149ea860c8fe98def29ecf02"),
        );
        c2.update_nonce(5);

        let c3 = block_state
            .get_mut_contract(Felt::from_hex_unchecked(
                "0x49d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7",
            ))
            .unwrap();
        c3.update_storage(
            Felt::from_hex_unchecked("0xe8fc4f1b6b3dc661208f9a8a5017a6c059098327e31518722e0a5c3a5a7e86"),
            Felt::from_hex_unchecked("0x1"),
        );
        c3.update_storage(
            Felt::from_hex_unchecked("0x6d56a3a16e0bf05482515c10fcf552437cdd1b7b409a6e8cec88c4b8fa03c13"),
            Felt::from_hex_unchecked("0x1"),
        );

        assert_eq!(
            block_state.state_root(),
            Felt::from_hex_unchecked("0xe005205a1327f3dff98074e528f7b96f30e0624a1dfcf571bdc81948d150a0")
        );
    }
}
