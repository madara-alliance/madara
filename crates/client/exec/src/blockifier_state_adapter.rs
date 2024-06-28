use std::collections::HashSet;
use std::sync::Arc;

use blockifier::execution::contract_class::ContractClass;
use blockifier::state::cached_state::CommitmentStateDiff;
use blockifier::state::errors::StateError;
use blockifier::state::state_api::{State, StateReader, StateResult};
use dc_db::storage_handler::StorageView;
use dc_db::DeoxysBackend;
use dp_block::BlockId;
use dp_class::to_blockifier_class;
use dp_convert::{ToFelt, ToStarkFelt};
use indexmap::IndexMap;
use starknet_api::core::{ClassHash, CompiledClassHash, ContractAddress, Nonce};
use starknet_api::hash::StarkFelt;
use starknet_api::state::StorageKey;

/// `BlockifierStateAdapter` is only use to re-executing or simulate transactions.
/// None of the setters should therefore change the storage persistently,
/// all changes are temporary stored in the struct and are discarded after the execution
pub struct BlockifierStateAdapter {
    backend: Arc<DeoxysBackend>,
    /// When this value is None, we are executing the genesis block.
    block_number: Option<u64>,
    storage_update: IndexMap<ContractAddress, IndexMap<StorageKey, StarkFelt>>,
    nonce_update: IndexMap<ContractAddress, Nonce>,
    class_hash_update: IndexMap<ContractAddress, ClassHash>,
    compiled_class_hash_update: IndexMap<ClassHash, CompiledClassHash>,
    contract_class_update: IndexMap<ClassHash, ContractClass>,
    visited_pcs: IndexMap<ClassHash, HashSet<usize>>,
}

impl BlockifierStateAdapter {
    pub fn new(backend: Arc<DeoxysBackend>, block_number: Option<u64>) -> Self {
        Self {
            backend,
            block_number,
            storage_update: IndexMap::default(),
            nonce_update: IndexMap::default(),
            class_hash_update: IndexMap::default(),
            compiled_class_hash_update: IndexMap::default(),
            contract_class_update: IndexMap::default(),
            visited_pcs: IndexMap::default(),
        }
    }
}

impl StateReader for BlockifierStateAdapter {
    fn get_storage_at(&mut self, contract_address: ContractAddress, key: StorageKey) -> StateResult<StarkFelt> {
        if *contract_address.key() == StarkFelt::ONE {
            let block_number = (*key.0.key()).try_into().map_err(|_| StateError::OldBlockHashNotProvided)?;

            return Ok(self
                .backend
                .mapping()
                .get_block_hash(&BlockId::Number(block_number))
                .map_err(|err| {
                    StateError::StateReadError(format!(
                        "Failed to retrieve block hash for block number {block_number}: {err:#}",
                    ))
                })?
                .ok_or(StateError::OldBlockHashNotProvided)?
                .to_stark_felt());
        }

        let Some(block_number) = self.block_number else { return Ok(StarkFelt::ZERO) };

        Ok(self
            .backend
            .contract_storage()
            .get_at(&(contract_address.to_felt(), key.to_felt()), block_number)
            .map_err(|err| {
                StateError::StateReadError(format!(
                    "Failed to retrieve storage value for contract {contract_address:#?} at key {key:#?}: {err:#}",
                ))
            })?
            .unwrap_or_default()
            .to_stark_felt())
    }

    fn get_nonce_at(&mut self, contract_address: ContractAddress) -> StateResult<Nonce> {
        if let Some(nonce) = self.nonce_update.get(&contract_address) {
            return Ok(*nonce);
        }
        let Some(block_number) = self.block_number else { return Ok(Nonce::default()) };

        Ok(self
            .backend
            .contract_nonces()
            .get_at(&contract_address.to_felt(), block_number)
            .map_err(|err| {
                StateError::StateReadError(format!(
                    "Failed to retrieve nonce for contract {contract_address:#?}: {err:#}",
                ))
            })?
            .unwrap_or_default())
    }

    fn get_class_hash_at(&mut self, contract_address: ContractAddress) -> StateResult<ClassHash> {
        if let Some(class_hash) = self.class_hash_update.get(&contract_address).cloned() {
            return Ok(class_hash);
        }
        let Some(block_number) = self.block_number else { return Ok(ClassHash::default()) };

        // Note that blockifier is fine with us returning ZERO as a class_hash if it is not found, they do the check on their end after

        Ok(ClassHash(
            self.backend
                .contract_class_hash()
                .get_at(&contract_address.to_felt(), block_number)
                .map_err(|err| {
                    StateError::StateReadError(format!(
                        "Failed to retrieve class hash for contract {:#}: {:#}",
                        contract_address.0.key(),
                        err
                    ))
                })?
                .unwrap_or_default()
                .to_stark_felt(),
        ))
    }

    fn get_compiled_contract_class(&mut self, class_hash: ClassHash) -> StateResult<ContractClass> {
        match self.contract_class_update.get(&class_hash) {
            Some(contract_class) => Ok(contract_class.clone()),
            None => match self.backend.compiled_contract_class().get(&class_hash.to_felt()) {
                Ok(Some(compiled_class)) => to_blockifier_class(compiled_class).map_err(StateError::ProgramError),
                _ => Err(StateError::UndeclaredClassHash(class_hash)),
            },
        }
    }

    fn get_compiled_class_hash(&mut self, class_hash: ClassHash) -> StateResult<CompiledClassHash> {
        match self.compiled_class_hash_update.get(&class_hash) {
            Some(compiled_class_hash) => Ok(*compiled_class_hash),
            None => self
                .backend
                .contract_class_hashes()
                .get(&class_hash.to_felt())
                .map_err(|_| {
                    StateError::StateReadError(format!(
                        "failed to retrive compiled class hash at class hash {:#}",
                        class_hash.0
                    ))
                })?
                .map(|felt| CompiledClassHash(felt.to_stark_felt()))
                .ok_or(StateError::UndeclaredClassHash(class_hash)),
        }
    }
}

impl State for BlockifierStateAdapter {
    fn set_storage_at(
        &mut self,
        contract_address: ContractAddress,
        key: StorageKey,
        value: StarkFelt,
    ) -> StateResult<()> {
        self.storage_update.entry(contract_address).or_default().insert(key, value);

        Ok(())
    }

    fn increment_nonce(&mut self, contract_address: ContractAddress) -> StateResult<()> {
        let nonce = self.get_nonce_at(contract_address)?.try_increment().map_err(StateError::StarknetApiError)?;

        self.nonce_update.insert(contract_address, nonce);

        Ok(())
    }

    fn set_class_hash_at(&mut self, contract_address: ContractAddress, class_hash: ClassHash) -> StateResult<()> {
        self.class_hash_update.insert(contract_address, class_hash);

        Ok(())
    }

    fn set_contract_class(&mut self, class_hash: ClassHash, contract_class: ContractClass) -> StateResult<()> {
        self.contract_class_update.insert(class_hash, contract_class);

        Ok(())
    }

    fn set_compiled_class_hash(
        &mut self,
        class_hash: ClassHash,
        compiled_class_hash: CompiledClassHash,
    ) -> StateResult<()> {
        self.compiled_class_hash_update.insert(class_hash, compiled_class_hash);

        Ok(())
    }

    fn add_visited_pcs(&mut self, class_hash: ClassHash, pcs: &HashSet<usize>) {
        self.visited_pcs.entry(class_hash).or_default().extend(pcs);
    }

    fn to_state_diff(&mut self) -> CommitmentStateDiff {
        CommitmentStateDiff {
            address_to_class_hash: self.class_hash_update.clone(),
            address_to_nonce: self.nonce_update.clone(),
            storage_updates: self.storage_update.clone(),
            class_hash_to_compiled_class_hash: self.compiled_class_hash_update.clone(),
        }
    }
}
