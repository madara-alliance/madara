use crate::BlockifierStateAdapter;
use blockifier::{
    execution::contract_class::ContractClass,
    state::{
        cached_state::StateMaps,
        state_api::{StateReader, StateResult},
    },
};
use mc_db::{db_block_id::DbBlockId, MadaraBackend};
use mp_convert::Felt;
use starknet_api::{
    core::{ClassHash, CompiledClassHash, ContractAddress, Nonce},
    state::StorageKey,
};
use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
};

#[derive(Debug)]
struct CacheByBlock {
    block_n: u64,
    state_diff: StateMaps,
    classes: HashMap<ClassHash, ContractClass>,
}

/// Special cache that allows us to execute blocks past what the db currently has saved. Only used for
/// block production.
/// We need this because when a block is produced, saving it do the database is done asynchronously by another task. This means
/// that we need to keep the state of the previous block around to execute the next one. We can only remove the cached state of the
/// previous blocks once we know they are imported into the database.
pub struct LayeredStateAdaptor {
    inner: BlockifierStateAdapter,
    cached_states_by_block_n: VecDeque<CacheByBlock>,
    backend: Arc<MadaraBackend>,
}
impl LayeredStateAdaptor {
    pub fn new(backend: Arc<MadaraBackend>) -> Result<Self, crate::Error> {
        let on_top_of_block_n = backend.get_latest_block_n()?;
        let block_number = on_top_of_block_n.map(|n| n + 1).unwrap_or(/* genesis */ 0);

        Ok(Self {
            inner: BlockifierStateAdapter::new(backend.clone(), block_number, on_top_of_block_n.map(DbBlockId::Number)),
            backend,
            cached_states_by_block_n: Default::default(),
        })
    }

    /// Currently executing block_n.
    pub fn block_n(&self) -> u64 {
        self.inner.block_number
    }

    /// Previous executing block_n. None means it is the genesis block.
    pub fn previous_block_n(&self) -> Option<u64> {
        self.inner.block_number.checked_sub(1)
    }

    fn remove_cache_before_including(&mut self, block_n: Option<u64>) {
        if let Some(block_n) = block_n {
            while self.cached_states_by_block_n.back().is_some_and(|cache| cache.block_n <= block_n) {
                let popped = self.cached_states_by_block_n.pop_back().expect("Checked that back exists just above.");
                tracing::debug!("Removed cache {:?} ", popped.block_n);
            }
        }
    }

    /// This will set the current executing block_n to the next block_n.
    pub fn finish_block(
        &mut self,
        state_diff: StateMaps,
        classes: HashMap<ClassHash, ContractClass>,
    ) -> Result<(), crate::Error> {
        let latest_db_block = self.backend.get_latest_block_n()?;
        // Remove outdated cache entries
        self.remove_cache_before_including(latest_db_block);

        // Push the current state to cache
        let block_n = self.block_n();
        tracing::debug!("Push to cache {block_n}");
        self.cached_states_by_block_n.push_front(CacheByBlock { block_n, state_diff, classes });

        // Update the inner state adaptor to update its block_n to the next one.
        self.inner = BlockifierStateAdapter::new(
            self.backend.clone(),
            block_n + 1,
            /* on top of */ latest_db_block.map(DbBlockId::Number),
        );

        Ok(())
    }
}

impl StateReader for LayeredStateAdaptor {
    fn get_storage_at(&self, contract_address: ContractAddress, key: StorageKey) -> StateResult<Felt> {
        for s in &self.cached_states_by_block_n {
            if let Some(el) = s.state_diff.storage.get(&(contract_address, key)) {
                return Ok(*el);
            }
        }
        self.inner.get_storage_at(contract_address, key)
    }
    fn get_nonce_at(&self, contract_address: ContractAddress) -> StateResult<Nonce> {
        for s in &self.cached_states_by_block_n {
            if let Some(el) = s.state_diff.nonces.get(&contract_address) {
                return Ok(*el);
            }
        }
        self.inner.get_nonce_at(contract_address)
    }
    fn get_class_hash_at(&self, contract_address: ContractAddress) -> StateResult<ClassHash> {
        for s in &self.cached_states_by_block_n {
            if let Some(el) = s.state_diff.class_hashes.get(&contract_address) {
                return Ok(*el);
            }
        }
        self.inner.get_class_hash_at(contract_address)
    }
    fn get_compiled_contract_class(&self, class_hash: ClassHash) -> StateResult<ContractClass> {
        for s in &self.cached_states_by_block_n {
            if let Some(el) = s.classes.get(&class_hash) {
                return Ok(el.clone());
            }
        }
        self.inner.get_compiled_contract_class(class_hash)
    }
    fn get_compiled_class_hash(&self, class_hash: ClassHash) -> StateResult<CompiledClassHash> {
        for s in &self.cached_states_by_block_n {
            if let Some(el) = s.state_diff.compiled_class_hashes.get(&class_hash) {
                return Ok(*el);
            }
        }
        self.inner.get_compiled_class_hash(class_hash)
    }
}
