use blockifier::execution::contract_class::ContractClass;
use blockifier::state::errors::StateError;
use blockifier::state::state_api::{StateReader, StateResult};
use dc_db::db_block_id::DbBlockId;
use dc_db::DeoxysBackend;
use dp_block::BlockId;
use dp_class::to_blockifier_class;
use dp_convert::{felt_to_u64, ToFelt};
use starknet_api::core::{ClassHash, CompiledClassHash, ContractAddress, Nonce};
use starknet_api::state::StorageKey;
use starknet_core::types::Felt;

/// `BlockifierStateAdapter` is only use to re-executing or simulate transactions.
/// All of the setters change the storage permanently. Use blockifier's `CachedState` for a cache.
pub struct BlockifierStateAdapter<'a> {
    backend: &'a DeoxysBackend,
    /// When this value is None, we are executing the genesis block.
    on_top_of_block_id: Option<DbBlockId>,
}

impl<'a> BlockifierStateAdapter<'a> {
    pub fn new(backend: &'a DeoxysBackend, on_top_of_block_id: Option<DbBlockId>) -> Self {
        Self { backend, on_top_of_block_id }
    }
}

impl<'a> StateReader for BlockifierStateAdapter<'a> {
    fn get_storage_at(&self, contract_address: ContractAddress, key: StorageKey) -> StateResult<Felt> {
        if *contract_address.key() == Felt::ONE {
            let block_number = felt_to_u64(key.0.key()).map_err(|_| StateError::OldBlockHashNotProvided)?;

            return Ok(self
                .backend
                .get_block_hash(&BlockId::Number(block_number))
                .map_err(|err| {
                    log::warn!("Failed to retrieve block hash for block number {block_number}: {err:#}");
                    StateError::StateReadError(
                        format!("Failed to retrieve block hash for block number {block_number}",),
                    )
                })?
                .ok_or(StateError::OldBlockHashNotProvided)?);
        }

        let Some(on_top_of_block_id) = self.on_top_of_block_id else { return Ok(Felt::ZERO) };

        Ok(self
            .backend
            .get_contract_storage_at(&on_top_of_block_id, &contract_address.to_felt(), &key.to_felt())
            .map_err(|err| {
                log::warn!(
                    "Failed to retrieve storage value for contract {contract_address:#?} at key {key:#?}: {err:#}"
                );
                StateError::StateReadError(format!(
                    "Failed to retrieve storage value for contract {contract_address:#?} at key {key:#?}",
                ))
            })?
            .unwrap_or(Felt::ZERO))
    }

    fn get_nonce_at(&self, contract_address: ContractAddress) -> StateResult<Nonce> {
        log::debug!("get_nonce_at for {:#?}", contract_address);
        let Some(on_top_of_block_id) = self.on_top_of_block_id else { return Ok(Nonce::default()) };

        Ok(Nonce(
            self.backend
                .get_contract_nonce_at(&on_top_of_block_id, &contract_address.to_felt())
                .map_err(|err| {
                    log::warn!("Failed to retrieve nonce for contract {contract_address:#?}: {err:#}");
                    StateError::StateReadError(format!("Failed to retrieve nonce for contract {contract_address:#?}",))
                })?
                .unwrap_or(Felt::ZERO),
        ))
    }

    fn get_class_hash_at(&self, contract_address: ContractAddress) -> StateResult<ClassHash> {
        log::debug!("get_class_hash_at for {:#?}", contract_address);
        let Some(on_top_of_block_id) = self.on_top_of_block_id else { return Ok(ClassHash::default()) };

        // Note that blockifier is fine with us returning ZERO as a class_hash if it is not found, they do the check on their end after
        Ok(ClassHash(
            self.backend
                .get_contract_class_hash_at(&on_top_of_block_id, &contract_address.to_felt())
                .map_err(|err| {
                    StateError::StateReadError(format!(
                        "Failed to retrieve class hash for contract {:#}: {:#}",
                        contract_address.0.key(),
                        err
                    ))
                })?
                .unwrap_or_default(),
        ))
    }

    fn get_compiled_contract_class(&self, class_hash: ClassHash) -> StateResult<ContractClass> {
        log::debug!("get_compiled_contract_class for {:#?}", class_hash);

        let Some(on_top_of_block_id) = self.on_top_of_block_id else {
            return Err(StateError::UndeclaredClassHash(class_hash));
        };

        let Some((_class_info, compiled_class)) =
            self.backend.get_class(&on_top_of_block_id, &class_hash.to_felt()).map_err(|err| {
                log::warn!("Failed to retrieve compiled class {class_hash:#}: {err:#}");
                StateError::StateReadError(format!("Failed to retrieve compiled class {class_hash:#}"))
            })?
        else {
            return Err(StateError::UndeclaredClassHash(class_hash));
        };

        to_blockifier_class(compiled_class).map_err(StateError::ProgramError)
    }

    fn get_compiled_class_hash(&self, class_hash: ClassHash) -> StateResult<CompiledClassHash> {
        log::debug!("get_compiled_contract_class for {:#?}", class_hash);

        let Some(on_top_of_block_id) = self.on_top_of_block_id else {
            return Err(StateError::UndeclaredClassHash(class_hash));
        };
        let Some(class_info) =
            self.backend.get_class_info(&on_top_of_block_id, &class_hash.to_felt()).map_err(|err| {
                log::warn!("Failed to retrieve compiled class hash {class_hash:#}: {err:#}");
                StateError::StateReadError(format!("Failed to retrieve compiled class hash {class_hash:#}",))
            })?
        else {
            return Err(StateError::UndeclaredClassHash(class_hash));
        };

        Ok(CompiledClassHash(class_info.compiled_class_hash))
    }
}

// impl<'a> State for BlockifierStateAdapter<'a> {
//     fn set_storage_at(
//         &mut self,
//         contract_address: ContractAddress,
//         key: StorageKey,
//         value: StarkFelt,
//     ) -> StateResult<()> {
//         self.storage_update.entry(contract_address).or_default().insert(key, value);

//         Ok(())
//     }

//     fn increment_nonce(&mut self, contract_address: ContractAddress) -> StateResult<()> {
//         let nonce = self.get_nonce_at(contract_address)?.try_increment().map_err(StateError::StarknetApiError)?;

//         self.nonce_update.insert(contract_address, nonce);

//         Ok(())
//     }

//     fn set_class_hash_at(&mut self, contract_address: ContractAddress, class_hash: ClassHash) -> StateResult<()> {
//         self.class_hash_update.insert(contract_address, class_hash);

//         Ok(())
//     }

//     fn set_contract_class(&mut self, class_hash: ClassHash, contract_class: ContractClass) -> StateResult<()> {
//         self.contract_class_update.insert(class_hash, contract_class);

//         Ok(())
//     }

//     fn set_compiled_class_hash(
//         &mut self,
//         class_hash: ClassHash,
//         compiled_class_hash: CompiledClassHash,
//     ) -> StateResult<()> {
//         self.compiled_class_hash_update.insert(class_hash, compiled_class_hash);

//         Ok(())
//     }

//     fn add_visited_pcs(&mut self, class_hash: ClassHash, pcs: &HashSet<usize>) {
//         self.visited_pcs.entry(class_hash).or_default().extend(pcs);
//     }

//     fn to_state_diff(&mut self) -> CommitmentStateDiff {
//         CommitmentStateDiff {
//             address_to_class_hash: self.class_hash_update.clone(),
//             address_to_nonce: self.nonce_update.clone(),
//             storage_updates: self.storage_update.clone(),
//             class_hash_to_compiled_class_hash: self.compiled_class_hash_update.clone(),
//         }
//     }
// }
