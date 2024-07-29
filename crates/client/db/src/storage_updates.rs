use crate::db_block_id::DbBlockId;
use crate::DeoxysBackend;
use crate::DeoxysStorageError;
use dp_block::header::PendingHeader;
use dp_block::{
    BlockId, BlockTag, DeoxysBlock, DeoxysMaybePendingBlock, DeoxysMaybePendingBlockInfo, DeoxysPendingBlock,
};
use dp_class::ConvertedClass;
use dp_state_update::{
    ContractStorageDiffItem, DeployedContractItem, NonceUpdate, ReplacedClassItem, StateDiff, StorageEntry,
};
use starknet_core::types::ContractClass;
use starknet_types_core::felt::Felt;
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct DbClassUpdate {
    pub class_hash: Felt,
    pub contract_class: ContractClass,
    pub compiled_class_hash: Felt,
}

impl DeoxysBackend {
    /// NB: This functions needs to run on the rayon thread pool
    pub fn store_block(
        &self,
        block: DeoxysMaybePendingBlock,
        state_diff: StateDiff,
        converted_classes: Vec<ConvertedClass>,
    ) -> Result<(), DeoxysStorageError> {
        let block_n = block.info.block_n();
        let state_diff_cpy = state_diff.clone();

        let task_block_db = || match block.info {
            DeoxysMaybePendingBlockInfo::Pending(info) => {
                self.block_db_store_pending(&DeoxysPendingBlock { info, inner: block.inner }, &state_diff_cpy)
            }
            DeoxysMaybePendingBlockInfo::NotPending(info) => {
                self.block_db_store_block(&DeoxysBlock { info, inner: block.inner }, &state_diff_cpy)
            }
        };

        let task_contract_db = || {
            // let nonces_from_deployed =
            //     state_diff.deployed_contracts.iter().map(|&DeployedContractItem { address, .. }| (address, Felt::ZERO));

            let nonces_from_updates =
                state_diff.nonces.into_iter().map(|NonceUpdate { contract_address, nonce }| (contract_address, nonce));

            let nonce_map: HashMap<Felt, Felt> = nonces_from_updates.collect();
            // let nonce_map: HashMap<Felt, Felt> = nonces_from_deployed.chain(nonces_from_updates).collect();

            let contract_class_updates_replaced = state_diff
                .replaced_classes
                .into_iter()
                .map(|ReplacedClassItem { contract_address, class_hash }| (contract_address, class_hash));

            let contract_class_updates_deployed = state_diff
                .deployed_contracts
                .into_iter()
                .map(|DeployedContractItem { address, class_hash }| (address, class_hash));

            let contract_class_updates =
                contract_class_updates_replaced.chain(contract_class_updates_deployed).collect::<Vec<_>>();
            let nonces_updates = nonce_map.into_iter().collect::<Vec<_>>();

            let storage_kv_updates = state_diff
                .storage_diffs
                .into_iter()
                .flat_map(|ContractStorageDiffItem { address, storage_entries }| {
                    storage_entries.into_iter().map(move |StorageEntry { key, value }| ((address, key), value))
                })
                .collect::<Vec<_>>();

            match block_n {
                None => self.contract_db_store_pending(&contract_class_updates, &nonces_updates, &storage_kv_updates),
                Some(block_n) => {
                    self.contract_db_store_block(block_n, &contract_class_updates, &nonces_updates, &storage_kv_updates)
                }
            }
        };

        let task_class_db = || {
            let (class_info_updates, compiled_class_updates): (Vec<_>, Vec<_>) = converted_classes
                .into_iter()
                .map(|ConvertedClass { class_infos, class_compiled }| (class_infos, class_compiled))
                .unzip();
            match block_n {
                None => self.class_db_store_pending(&class_info_updates, &compiled_class_updates),
                Some(block_n) => self.class_db_store_block(block_n, &class_info_updates, &compiled_class_updates),
            }
        };

        let ((r1, r2), r3) = rayon::join(|| rayon::join(task_block_db, task_contract_db), task_class_db);

        r1.and(r2).and(r3)
    }

    pub fn clear_pending_block(&self) -> Result<(), DeoxysStorageError> {
        self.block_db_clear_pending()?;
        self.contract_db_clear_pending()?;
        self.class_db_clear_pending()?;
        Ok(())
    }

    /// This function creates the pending block if it is not found.
    ///
    /// # Arguments
    /// - `create_header` is supplied the block hash as argument.
    // TODO(docs): unsure how to clarify the `create_header` signature but having an untyped unnamed Felt here sux
    pub fn get_or_create_pending_block(
        &self,
        create_header: impl FnOnce(Felt) -> PendingHeader,
    ) -> Result<DeoxysMaybePendingBlock, DeoxysStorageError> {
        match self.get_block(&DbBlockId::Pending)? {
            Some(block) => Ok(block),
            None => {
                // No pending block: we create one :)

                let block_info = self
                    .get_block_info(&BlockId::Tag(BlockTag::Latest))?
                    .ok_or(DeoxysStorageError::PendingCreationNoGenesis)?;
                let previous = block_info.as_nonpending().ok_or(DeoxysStorageError::InvalidBlockNumber)?; // TODO(merge): change with charpa's error when rebasing on main

                Ok(DeoxysPendingBlock::new_empty(create_header(previous.block_hash)).into())
            }
        }
    }
}
