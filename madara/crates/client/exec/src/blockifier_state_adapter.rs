use blockifier::execution::contract_class::ContractClass;
use blockifier::state::errors::StateError;
use blockifier::state::state_api::{StateReader, StateResult};
use mc_db::db_block_id::DbBlockId;
use mc_db::MadaraBackend;
use mp_convert::ToFelt;
use starknet_api::core::{ClassHash, CompiledClassHash, ContractAddress, Nonce};
use starknet_api::state::StorageKey;
use starknet_types_core::felt::Felt;
use std::sync::Arc;

/// Adapter for the db queries made by blockifier.
///
/// There is no actual mutable logic here - when using block production, the actual key value
/// changes in db are evaluated at the end only from the produced state diff.
pub struct BlockifierStateAdapter {
    backend: Arc<MadaraBackend>,
    /// When this value is None, we are executing the genesis block.
    pub on_top_of_block_id: Option<DbBlockId>,
    pub block_number: u64,
}

impl BlockifierStateAdapter {
    pub fn new(backend: Arc<MadaraBackend>, block_number: u64, on_top_of_block_id: Option<DbBlockId>) -> Self {
        Self { backend, on_top_of_block_id, block_number }
    }

    pub fn is_l1_to_l2_message_nonce_consumed(&self, nonce: u64) -> StateResult<bool> {
        let value = self
            .backend
            .get_l1_handler_txn_hash_by_core_contract_nonce(nonce)
            .map_err(|err| {
                StateError::StateReadError(format!(
                    "Failed to l1 handler txn hash by core contract nonce: on={:?}, nonce={nonce}: {err:#}",
                    self.on_top_of_block_id
                ))
            })?
            .is_some();

        tracing::debug!(
            "get_l1_handler_txn_hash_by_core_contract_nonce: on={:?}, nonce={nonce} => {value}",
            self.on_top_of_block_id,
        );
        Ok(value)
    }
}

// TODO: mapping StateErrors InternalServerError in execution RPC endpoints is not properly handled.
// It is however properly handled for transaction validator.
impl StateReader for BlockifierStateAdapter {
    fn get_storage_at(&self, contract_address: ContractAddress, key: StorageKey) -> StateResult<Felt> {
        let value = match self.on_top_of_block_id {
            Some(on_top_of_block_id) => self
                .backend
                .get_contract_storage_at(&on_top_of_block_id, &contract_address.to_felt(), &key.to_felt())
                .map_err(|err| {
                    StateError::StateReadError(format!(
                        "Failed to retrieve storage value: on={:?}, contract_address={:#x} key={:#x}: {err:#}",
                        self.on_top_of_block_id,
                        contract_address.to_felt(),
                        key.to_felt(),
                    ))
                })?
                .unwrap_or(Felt::ZERO),
            None => Felt::ZERO,
        };

        tracing::debug!(
            "get_storage_at: on={:?}, contract_address={:#x} key={:#x} => {value:#x}",
            self.on_top_of_block_id,
            contract_address.to_felt(),
            key.to_felt(),
        );

        Ok(value)
    }

    fn get_nonce_at(&self, contract_address: ContractAddress) -> StateResult<Nonce> {
        let value = match self.on_top_of_block_id {
            Some(on_top_of_block_id) => self
                .backend
                .get_contract_nonce_at(&on_top_of_block_id, &contract_address.to_felt())
                .map_err(|err| {
                    StateError::StateReadError(format!(
                        "Failed to retrieve nonce: on={:?}, contract_address={:#x}: {err:#}",
                        self.on_top_of_block_id,
                        contract_address.to_felt(),
                    ))
                })?
                .unwrap_or(Felt::ZERO),
            None => Felt::ZERO,
        };

        tracing::debug!(
            "get_nonce_at: on={:?}, contract_address={:#x} => {value:#x}",
            self.on_top_of_block_id,
            contract_address.to_felt(),
        );

        Ok(Nonce(value))
    }

    /// Blockifier expects us to return 0x0 if the contract is not deployed.
    fn get_class_hash_at(&self, contract_address: ContractAddress) -> StateResult<ClassHash> {
        let value = match self.on_top_of_block_id {
            Some(on_top_of_block_id) => self
                .backend
                .get_contract_class_hash_at(&on_top_of_block_id, &contract_address.to_felt())
                .map_err(|err| {
                    StateError::StateReadError(format!(
                        "Failed to retrieve class_hash: on={:?}, contract_address={:#x}: {err:#}",
                        self.on_top_of_block_id,
                        contract_address.to_felt(),
                    ))
                })?
                .unwrap_or(Felt::ZERO),
            None => Felt::ZERO,
        };

        tracing::debug!(
            "get_class_hash_at: on={:?}, contract_address={:#x} => {value:#x}",
            self.on_top_of_block_id,
            contract_address.to_felt(),
        );

        Ok(ClassHash(value))
    }

    fn get_compiled_contract_class(&self, class_hash: ClassHash) -> StateResult<ContractClass> {
        let value = match self.on_top_of_block_id {
            Some(on_top_of_block_id) => {
                self.backend.get_converted_class(&on_top_of_block_id, &class_hash.to_felt()).map_err(|err| {
                    StateError::StateReadError(format!(
                        "Failed to retrieve class_hash: on={:?}, class_hash={:#x}: {err:#}",
                        self.on_top_of_block_id,
                        class_hash.to_felt(),
                    ))
                })?
            }
            None => None,
        };

        let value = value.ok_or(StateError::UndeclaredClassHash(class_hash))?;

        tracing::debug!(
            "get_compiled_contract_class: on={:?}, class_hash={:#x}",
            self.on_top_of_block_id,
            class_hash.to_felt()
        );

        value.to_blockifier_class().map_err(|err| {
            StateError::StateReadError(format!("Failed to convert class {class_hash:#} to blockifier format: {err:#}"))
        })
    }

    fn get_compiled_class_hash(&self, class_hash: ClassHash) -> StateResult<CompiledClassHash> {
        let value = match self.on_top_of_block_id {
            Some(on_top_of_block_id) => {
                self.backend.get_class_info(&on_top_of_block_id, &class_hash.to_felt()).map_err(|err| {
                    StateError::StateReadError(format!(
                        "Failed to retrieve class_hash: on={:?}, class_hash={:#x}: {err:#}",
                        self.on_top_of_block_id,
                        class_hash.to_felt(),
                    ))
                })?
            }
            None => None,
        };

        let value = value.and_then(|c| c.compiled_class_hash()).ok_or_else(|| {
            StateError::StateReadError(format!(
                "Class does not have a compiled class hash: on={:?}, class_hash={:#x}",
                self.on_top_of_block_id,
                class_hash.to_felt(),
            ))
        })?;

        tracing::debug!(
            "get_compiled_class_hash: on={:?}, class_hash={:#x} => {value:#x}",
            self.on_top_of_block_id,
            class_hash.to_felt(),
        );

        Ok(CompiledClassHash(value))
    }
}
