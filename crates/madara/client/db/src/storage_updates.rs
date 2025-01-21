use crate::db_block_id::DbBlockId;
use crate::MadaraBackend;
use crate::MadaraStorageError;
use blockifier::bouncer::BouncerWeights;
use mp_block::VisitedSegments;
use mp_block::{MadaraBlock, MadaraMaybePendingBlock, MadaraMaybePendingBlockInfo, MadaraPendingBlock};
use mp_bloom_filter::EventBloomWriter;
use mp_class::ConvertedClass;
use mp_state_update::{
    ContractStorageDiffItem, DeployedContractItem, NonceUpdate, ReplacedClassItem, StateDiff, StorageEntry,
};
use starknet_types_core::felt::Felt;
use std::collections::HashMap;

impl MadaraBackend {
    /// NB: This functions needs to run on the rayon thread pool
    pub fn store_block(
        &self,
        block: MadaraMaybePendingBlock,
        state_diff: StateDiff,
        converted_classes: Vec<ConvertedClass>,
        events_bloom: Option<EventBloomWriter>,
        visited_segments: Option<VisitedSegments>,
        bouncer_weights: Option<BouncerWeights>,
    ) -> Result<(), MadaraStorageError> {
        let block_n = block.info.block_n();
        let state_diff_cpy = state_diff.clone();

        // Clear in every case, even when storing a pending block
        self.clear_pending_block()?;

        let task_block_db = || match block.info {
            MadaraMaybePendingBlockInfo::Pending(info) => self.block_db_store_pending(
                &MadaraPendingBlock { info, inner: block.inner },
                &state_diff_cpy,
                visited_segments,
                bouncer_weights,
            ),
            MadaraMaybePendingBlockInfo::NotPending(info) => {
                self.block_db_store_block(&MadaraBlock { info, inner: block.inner }, &state_diff_cpy, events_bloom)
            }
        };

        let task_contract_db = || {
            // let nonces_from_deployed =
            //     state_diff.deployed_contracts.iter().map(|&DeployedContractItem { address, .. }| (address, Felt::ZERO));

            let nonces_from_updates =
                state_diff.nonces.into_iter().map(|NonceUpdate { contract_address, nonce }| (contract_address, nonce));

            // let nonce_map: HashMap<Felt, Felt> = nonces_from_deployed.chain(nonces_from_updates).collect(); // set nonce to zero when contract deployed
            let nonce_map: HashMap<Felt, Felt> = nonces_from_updates.collect();

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

        let task_class_db = || match block_n {
            None => self.class_db_store_pending(&converted_classes),
            Some(block_n) => self.class_db_store_block(block_n, &converted_classes),
        };

        let ((r1, r2), r3) = rayon::join(|| rayon::join(task_block_db, task_contract_db), task_class_db);

        r1.and(r2).and(r3)?;

        self.snapshots.set_new_head(DbBlockId::from_block_n(block_n));
        Ok(())
    }

    pub fn clear_pending_block(&self) -> Result<(), MadaraStorageError> {
        self.block_db_clear_pending()?;
        self.contract_db_clear_pending()?;
        self.class_db_clear_pending()?;
        Ok(())
    }
}
