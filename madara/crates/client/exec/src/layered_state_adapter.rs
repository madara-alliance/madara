use crate::metrics::metrics as exec_metrics;
use crate::BlockifierStateAdapter;
use anyhow::Context;
use blockifier::{
    execution::contract_class::RunnableCompiledClass,
    state::{
        cached_state::StateMaps,
        errors::StateError,
        state_api::{StateReader, StateResult},
    },
};
use mc_db::{rocksdb::RocksDBStorage, ExecutionReadCacheConfig, MadaraBackend, MadaraStorageRead};
use mp_block::header::GasPrices;
use mp_convert::Felt;
use starknet_api::{
    contract_class::ContractClass as ApiContractClass,
    core::{ClassHash, CompiledClassHash, ContractAddress, Nonce},
    state::StorageKey,
};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    mem::size_of,
    sync::{Arc, RwLock},
};

#[derive(Debug)]
struct CacheByBlock {
    block_n: u64,
    state_diff: StateMaps,
    classes: HashMap<ClassHash, ApiContractClass>,
    l1_to_l2_messages: HashSet<u64>,
}

mod read_cache_kind {
    pub const STORAGE: &str = "storage";
    pub const NONCE: &str = "nonce";
    pub const CLASS_HASH: &str = "class_hash";
    pub const COMPILED_CLASS_HASH: &str = "compiled_class_hash";
    pub const COMPILED_CLASS_HASH_V2: &str = "compiled_class_hash_v2";
}

type StorageEntryKey = (ContractAddress, StorageKey);
type ContractEntryKey = ContractAddress;
type ClassEntryKey = ClassHash;

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
enum CacheKey {
    Storage(StorageEntryKey),
    Nonce(ContractEntryKey),
    ClassHash(ContractEntryKey),
    CompiledClassHash(ClassEntryKey),
    CompiledClassHashV2(ClassEntryKey),
}

#[derive(Debug, Clone)]
struct CacheEntry<T: Clone> {
    value: T,
    seq: u64,
}

#[derive(Debug)]
struct CacheQueueEntry {
    key: CacheKey,
    seq: u64,
}

#[derive(Debug, Default)]
struct ExecutionReadCacheInner {
    storage: HashMap<StorageEntryKey, CacheEntry<Felt>>,
    nonces: HashMap<ContractEntryKey, CacheEntry<Nonce>>,
    class_hashes: HashMap<ContractEntryKey, CacheEntry<ClassHash>>,
    cached_class_hashes: HashSet<ClassEntryKey>,
    compiled_class_hashes: HashMap<ClassEntryKey, CacheEntry<CompiledClassHash>>,
    compiled_class_hashes_v2: HashMap<ClassEntryKey, CacheEntry<CompiledClassHash>>,
    order: VecDeque<CacheQueueEntry>,
    current_bytes: usize,
    next_seq: u64,
}

#[derive(Debug)]
struct ExecutionReadCache {
    all_contracts: bool,
    contracts: HashSet<ContractAddress>,
    max_bytes: usize,
    inner: RwLock<ExecutionReadCacheInner>,
}

impl ExecutionReadCache {
    fn from_config(config: &ExecutionReadCacheConfig) -> Option<Self> {
        if !config.enabled || config.max_memory_bytes == 0 {
            return None;
        }

        let contracts = config.contracts.iter().copied().collect::<HashSet<_>>();
        if config.all_contracts {
            tracing::info!("Execution read cache enabled for all contracts (max_bytes={}).", config.max_memory_bytes);
        } else if contracts.is_empty() {
            tracing::warn!(
                "Execution read cache enabled but no contract allowlist provided (max_bytes={}).",
                config.max_memory_bytes
            );
        } else {
            tracing::info!(
                "Execution read cache enabled for {} contracts (max_bytes={}).",
                contracts.len(),
                config.max_memory_bytes
            );
        }

        exec_metrics().record_read_cache_size_bytes(0);

        Some(Self {
            all_contracts: config.all_contracts,
            contracts,
            max_bytes: config.max_memory_bytes,
            inner: RwLock::new(ExecutionReadCacheInner::default()),
        })
    }

    fn is_contract_enabled(&self, contract_address: ContractAddress) -> bool {
        self.all_contracts || self.contracts.contains(&contract_address)
    }

    fn should_cache_class_hash(&self, class_hash: ClassHash) -> bool {
        if self.all_contracts {
            return true;
        }
        let guard = self.inner.read().expect("Poisoned execution read cache lock");
        guard.cached_class_hashes.contains(&class_hash)
    }

    fn get_storage(&self, contract_address: ContractAddress, key: StorageKey) -> Option<Felt> {
        let guard = self.inner.read().expect("Poisoned execution read cache lock");
        guard.storage.get(&(contract_address, key)).map(|entry| entry.value)
    }

    fn get_nonce(&self, contract_address: ContractAddress) -> Option<Nonce> {
        let guard = self.inner.read().expect("Poisoned execution read cache lock");
        guard.nonces.get(&contract_address).map(|entry| entry.value)
    }

    fn get_class_hash(&self, contract_address: ContractAddress) -> Option<ClassHash> {
        let guard = self.inner.read().expect("Poisoned execution read cache lock");
        guard.class_hashes.get(&contract_address).map(|entry| entry.value)
    }

    fn get_compiled_class_hash(&self, class_hash: ClassHash) -> Option<CompiledClassHash> {
        let guard = self.inner.read().expect("Poisoned execution read cache lock");
        guard.compiled_class_hashes.get(&class_hash).map(|entry| entry.value)
    }

    fn get_compiled_class_hash_v2(&self, class_hash: ClassHash) -> Option<CompiledClassHash> {
        let guard = self.inner.read().expect("Poisoned execution read cache lock");
        guard.compiled_class_hashes_v2.get(&class_hash).map(|entry| entry.value)
    }

    fn insert_storage_value(&self, contract_address: ContractAddress, key: StorageKey, value: Felt) {
        if !self.is_contract_enabled(contract_address) {
            return;
        }
        let mut guard = self.inner.write().expect("Poisoned execution read cache lock");
        guard.insert_storage(contract_address, key, value);
        exec_metrics().record_read_cache_size_bytes(guard.current_bytes as u64);
    }

    fn insert_nonce_value(&self, contract_address: ContractAddress, value: Nonce) {
        if !self.is_contract_enabled(contract_address) {
            return;
        }
        let mut guard = self.inner.write().expect("Poisoned execution read cache lock");
        guard.insert_nonce(contract_address, value);
        exec_metrics().record_read_cache_size_bytes(guard.current_bytes as u64);
    }

    fn insert_class_hash_value(&self, contract_address: ContractAddress, value: ClassHash) {
        if !self.is_contract_enabled(contract_address) {
            return;
        }
        let mut guard = self.inner.write().expect("Poisoned execution read cache lock");
        guard.insert_class_hash(contract_address, value);
        guard.cached_class_hashes.insert(value);
        exec_metrics().record_read_cache_size_bytes(guard.current_bytes as u64);
    }

    fn insert_compiled_class_hash_value(&self, class_hash: ClassHash, value: CompiledClassHash) {
        if !self.should_cache_class_hash(class_hash) {
            return;
        }
        let mut guard = self.inner.write().expect("Poisoned execution read cache lock");
        guard.insert_compiled_class_hash(class_hash, value);
        exec_metrics().record_read_cache_size_bytes(guard.current_bytes as u64);
    }

    fn insert_compiled_class_hash_v2_value(&self, class_hash: ClassHash, value: CompiledClassHash) {
        if !self.should_cache_class_hash(class_hash) {
            return;
        }
        let mut guard = self.inner.write().expect("Poisoned execution read cache lock");
        guard.insert_compiled_class_hash_v2(class_hash, value);
        exec_metrics().record_read_cache_size_bytes(guard.current_bytes as u64);
    }

    fn apply_state_diff(&self, state_diff: &StateMaps) {
        let mut guard = self.inner.write().expect("Poisoned execution read cache lock");

        let mut allowed_class_hashes = HashSet::new();

        for (contract_address, class_hash) in &state_diff.class_hashes {
            if self.is_contract_enabled(*contract_address) {
                guard.insert_class_hash(*contract_address, *class_hash);
                allowed_class_hashes.insert(*class_hash);
                guard.cached_class_hashes.insert(*class_hash);
            }
        }

        for ((contract_address, key), value) in &state_diff.storage {
            if self.is_contract_enabled(*contract_address) {
                guard.insert_storage(*contract_address, *key, *value);
            }
        }

        for (contract_address, nonce) in &state_diff.nonces {
            if self.is_contract_enabled(*contract_address) {
                guard.insert_nonce(*contract_address, *nonce);
            }
        }

        if self.all_contracts {
            for (class_hash, compiled_hash) in &state_diff.compiled_class_hashes {
                guard.insert_compiled_class_hash(*class_hash, *compiled_hash);
            }
        } else if !allowed_class_hashes.is_empty() {
            for (class_hash, compiled_hash) in &state_diff.compiled_class_hashes {
                if allowed_class_hashes.contains(class_hash) {
                    guard.insert_compiled_class_hash(*class_hash, *compiled_hash);
                }
            }
        }

        exec_metrics().record_read_cache_size_bytes(guard.current_bytes as u64);
    }

    fn evict_if_needed(&self) {
        let mut guard = self.inner.write().expect("Poisoned execution read cache lock");
        let evicted = guard.evict_if_needed(self.max_bytes);
        exec_metrics().record_read_cache_size_bytes(guard.current_bytes as u64);
        if evicted > 0 {
            tracing::debug!("Execution read cache evicted {evicted} entries (size_bytes={}).", guard.current_bytes);
        }
    }
}

impl ExecutionReadCacheInner {
    const STORAGE_ENTRY_SIZE: usize = size_of::<CacheEntry<Felt>>() + size_of::<StorageEntryKey>();
    const NONCE_ENTRY_SIZE: usize = size_of::<CacheEntry<Nonce>>() + size_of::<ContractEntryKey>();
    const CLASS_HASH_ENTRY_SIZE: usize = size_of::<CacheEntry<ClassHash>>() + size_of::<ContractEntryKey>();
    const COMPILED_CLASS_HASH_ENTRY_SIZE: usize =
        size_of::<CacheEntry<CompiledClassHash>>() + size_of::<ClassEntryKey>();
    const ORDER_ENTRY_SIZE: usize = size_of::<CacheQueueEntry>();

    fn next_seq(&mut self) -> u64 {
        let seq = self.next_seq;
        self.next_seq = self.next_seq.wrapping_add(1);
        seq
    }

    fn insert_storage(&mut self, contract_address: ContractAddress, key: StorageKey, value: Felt) {
        let entry_size = Self::STORAGE_ENTRY_SIZE;
        let seq = self.next_seq();
        let cache_key = (contract_address, key);
        if self.storage.insert(cache_key, CacheEntry { value, seq }).is_some() {
            self.current_bytes = self.current_bytes.saturating_sub(entry_size);
        }
        self.current_bytes = self.current_bytes.saturating_add(entry_size);
        self.order.push_back(CacheQueueEntry { key: CacheKey::Storage(cache_key), seq });
        self.current_bytes = self.current_bytes.saturating_add(Self::ORDER_ENTRY_SIZE);
    }

    fn insert_nonce(&mut self, contract_address: ContractAddress, value: Nonce) {
        let entry_size = Self::NONCE_ENTRY_SIZE;
        let seq = self.next_seq();
        if self.nonces.insert(contract_address, CacheEntry { value, seq }).is_some() {
            self.current_bytes = self.current_bytes.saturating_sub(entry_size);
        }
        self.current_bytes = self.current_bytes.saturating_add(entry_size);
        self.order.push_back(CacheQueueEntry { key: CacheKey::Nonce(contract_address), seq });
        self.current_bytes = self.current_bytes.saturating_add(Self::ORDER_ENTRY_SIZE);
    }

    fn insert_class_hash(&mut self, contract_address: ContractAddress, value: ClassHash) {
        let entry_size = Self::CLASS_HASH_ENTRY_SIZE;
        let seq = self.next_seq();
        if self.class_hashes.insert(contract_address, CacheEntry { value, seq }).is_some() {
            self.current_bytes = self.current_bytes.saturating_sub(entry_size);
        }
        self.current_bytes = self.current_bytes.saturating_add(entry_size);
        self.order.push_back(CacheQueueEntry { key: CacheKey::ClassHash(contract_address), seq });
        self.current_bytes = self.current_bytes.saturating_add(Self::ORDER_ENTRY_SIZE);
    }

    fn insert_compiled_class_hash(&mut self, class_hash: ClassHash, value: CompiledClassHash) {
        let entry_size = Self::COMPILED_CLASS_HASH_ENTRY_SIZE;
        let seq = self.next_seq();
        if self.compiled_class_hashes.insert(class_hash, CacheEntry { value, seq }).is_some() {
            self.current_bytes = self.current_bytes.saturating_sub(entry_size);
        }
        self.current_bytes = self.current_bytes.saturating_add(entry_size);
        self.order.push_back(CacheQueueEntry { key: CacheKey::CompiledClassHash(class_hash), seq });
        self.current_bytes = self.current_bytes.saturating_add(Self::ORDER_ENTRY_SIZE);
    }

    fn insert_compiled_class_hash_v2(&mut self, class_hash: ClassHash, value: CompiledClassHash) {
        let entry_size = Self::COMPILED_CLASS_HASH_ENTRY_SIZE;
        let seq = self.next_seq();
        if self.compiled_class_hashes_v2.insert(class_hash, CacheEntry { value, seq }).is_some() {
            self.current_bytes = self.current_bytes.saturating_sub(entry_size);
        }
        self.current_bytes = self.current_bytes.saturating_add(entry_size);
        self.order.push_back(CacheQueueEntry { key: CacheKey::CompiledClassHashV2(class_hash), seq });
        self.current_bytes = self.current_bytes.saturating_add(Self::ORDER_ENTRY_SIZE);
    }

    fn evict_if_needed(&mut self, max_bytes: usize) -> usize {
        let mut evicted = 0;
        while self.current_bytes > max_bytes {
            let Some(entry) = self.order.pop_front() else { break };
            self.current_bytes = self.current_bytes.saturating_sub(Self::ORDER_ENTRY_SIZE);
            if self.remove_if_match(entry) {
                evicted += 1;
            }
        }
        evicted
    }

    fn remove_if_match(&mut self, entry: CacheQueueEntry) -> bool {
        match entry.key {
            CacheKey::Storage(cache_key) => {
                if let Some(existing) = self.storage.get(&cache_key) {
                    if existing.seq == entry.seq {
                        let _ = self.storage.remove(&cache_key).expect("entry exists");
                        self.current_bytes = self.current_bytes.saturating_sub(Self::STORAGE_ENTRY_SIZE);
                        return true;
                    }
                }
            }
            CacheKey::Nonce(contract_address) => {
                if let Some(existing) = self.nonces.get(&contract_address) {
                    if existing.seq == entry.seq {
                        let _ = self.nonces.remove(&contract_address).expect("entry exists");
                        self.current_bytes = self.current_bytes.saturating_sub(Self::NONCE_ENTRY_SIZE);
                        return true;
                    }
                }
            }
            CacheKey::ClassHash(contract_address) => {
                if let Some(existing) = self.class_hashes.get(&contract_address) {
                    if existing.seq == entry.seq {
                        let _ = self.class_hashes.remove(&contract_address).expect("entry exists");
                        self.current_bytes = self.current_bytes.saturating_sub(Self::CLASS_HASH_ENTRY_SIZE);
                        return true;
                    }
                }
            }
            CacheKey::CompiledClassHash(class_hash) => {
                if let Some(existing) = self.compiled_class_hashes.get(&class_hash) {
                    if existing.seq == entry.seq {
                        let _ = self.compiled_class_hashes.remove(&class_hash).expect("entry exists");
                        self.current_bytes = self.current_bytes.saturating_sub(Self::COMPILED_CLASS_HASH_ENTRY_SIZE);
                        return true;
                    }
                }
            }
            CacheKey::CompiledClassHashV2(class_hash) => {
                if let Some(existing) = self.compiled_class_hashes_v2.get(&class_hash) {
                    if existing.seq == entry.seq {
                        let _ = self.compiled_class_hashes_v2.remove(&class_hash).expect("entry exists");
                        self.current_bytes = self.current_bytes.saturating_sub(Self::COMPILED_CLASS_HASH_ENTRY_SIZE);
                        return true;
                    }
                }
            }
        }
        false
    }
}

/// Special cache that allows us to execute blocks past what the db currently has saved. Only used for
/// block production.
/// We need this because when a block is produced, saving it do the database is done asynchronously by another task. This means
/// that we need to keep the state of the previous block around to execute the next one. We can only remove the cached state of the
/// previous blocks once we know they are imported into the database.
pub struct LayeredStateAdapter<D: MadaraStorageRead = RocksDBStorage> {
    inner: BlockifierStateAdapter<D>,
    gas_prices: GasPrices,
    cached_states_by_block_n: VecDeque<CacheByBlock>,
    read_cache: Option<ExecutionReadCache>,
}

impl<D: MadaraStorageRead> LayeredStateAdapter<D> {
    pub fn new(backend: Arc<MadaraBackend<D>>) -> Result<Self, crate::Error> {
        let view = backend.view_on_latest_confirmed();
        let block_number = view.latest_block_n().map(|n| n + 1).unwrap_or(/* genesis */ 0);

        let l1_gas_quote = backend
            .get_last_l1_gas_quote()
            .context("No L1 gas quote available. Ensure that the L1 gas quote is set before calculating gas prices.")?;

        let gas_prices = if let Some(block) = view.block_view_on_latest_confirmed() {
            let block_info = block.get_block_info()?;
            let previous_strk_l2_gas_price = block_info.header.gas_prices.strk_l2_gas_price;
            let previous_l2_gas_used = block_info.total_l2_gas_used;

            backend.calculate_gas_prices(&l1_gas_quote, previous_strk_l2_gas_price, previous_l2_gas_used)?
        } else {
            backend.calculate_gas_prices(&l1_gas_quote, 0, 0)?
        };

        Ok(Self {
            inner: BlockifierStateAdapter::new(view, block_number),
            gas_prices,
            cached_states_by_block_n: Default::default(),
            read_cache: ExecutionReadCache::from_config(backend.execution_read_cache_config()),
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

    pub fn evict_read_cache_if_needed(&self) {
        if let Some(read_cache) = &self.read_cache {
            read_cache.evict_if_needed();
        }
    }

    fn remove_cache_before_including(&mut self, block_n: Option<u64>) {
        if let Some(block_n) = block_n {
            while self.cached_states_by_block_n.back().is_some_and(|cache| cache.block_n <= block_n) {
                let popped = self.cached_states_by_block_n.pop_back().expect("Checked that back exists just above.");
                if let Some(read_cache) = &self.read_cache {
                    read_cache.apply_state_diff(&popped.state_diff);
                }
                tracing::debug!("Removed cache {:?} ", popped.block_n);
            }
        }
    }

    /// This will set the current executing block_n to the next block_n.
    /// l1_to_l2_messages: list of consumed core contract nonces. We need to keep track of those to be absolutely sure we
    /// don't duplicate a transaction.
    pub fn finish_block(
        &mut self,
        state_diff: StateMaps,
        classes: HashMap<ClassHash, ApiContractClass>,
        l1_to_l2_messages: HashSet<u64>,
    ) -> Result<(), crate::Error> {
        let new_view = self.inner.view.backend().view_on_latest_confirmed();
        let latest_db_block = new_view.latest_block_n();

        // Remove outdated cache entries
        self.remove_cache_before_including(latest_db_block);

        // Push the current state to cache
        let block_n = self.block_n();
        tracing::debug!("Push to cache {block_n}");
        self.cached_states_by_block_n.push_front(CacheByBlock { block_n, state_diff, classes, l1_to_l2_messages });

        // Update the inner state adaptor to update its block_n to the next one.
        self.inner = BlockifierStateAdapter::new(new_view, block_n + 1);
        self.evict_read_cache_if_needed();

        Ok(())
    }

    pub fn latest_gas_prices(&self) -> &GasPrices {
        &self.gas_prices
    }

    pub fn is_l1_to_l2_message_nonce_consumed(&self, nonce: u64) -> StateResult<bool> {
        if self.cached_states_by_block_n.iter().any(|s| s.l1_to_l2_messages.contains(&nonce)) {
            return Ok(true);
        }
        self.inner.is_l1_to_l2_message_nonce_consumed(nonce)
    }
}

impl<D: MadaraStorageRead> StateReader for LayeredStateAdapter<D> {
    fn get_storage_at(&self, contract_address: ContractAddress, key: StorageKey) -> StateResult<Felt> {
        if let Some(el) =
            self.cached_states_by_block_n.iter().find_map(|s| s.state_diff.storage.get(&(contract_address, key)))
        {
            return Ok(*el);
        }
        if let Some(read_cache) = &self.read_cache {
            if read_cache.is_contract_enabled(contract_address) {
                if let Some(value) = read_cache.get_storage(contract_address, key) {
                    exec_metrics().record_read_cache_hit(read_cache_kind::STORAGE);
                    return Ok(value);
                }
                exec_metrics().record_read_cache_miss(read_cache_kind::STORAGE);
            }
        }
        let value = self.inner.get_storage_at(contract_address, key)?;
        if let Some(read_cache) = &self.read_cache {
            read_cache.insert_storage_value(contract_address, key, value);
        }
        Ok(value)
    }
    fn get_nonce_at(&self, contract_address: ContractAddress) -> StateResult<Nonce> {
        if let Some(el) = self.cached_states_by_block_n.iter().find_map(|s| s.state_diff.nonces.get(&contract_address))
        {
            return Ok(*el);
        }
        if let Some(read_cache) = &self.read_cache {
            if read_cache.is_contract_enabled(contract_address) {
                if let Some(value) = read_cache.get_nonce(contract_address) {
                    exec_metrics().record_read_cache_hit(read_cache_kind::NONCE);
                    return Ok(value);
                }
                exec_metrics().record_read_cache_miss(read_cache_kind::NONCE);
            }
        }
        let value = self.inner.get_nonce_at(contract_address)?;
        if let Some(read_cache) = &self.read_cache {
            read_cache.insert_nonce_value(contract_address, value);
        }
        Ok(value)
    }
    fn get_class_hash_at(&self, contract_address: ContractAddress) -> StateResult<ClassHash> {
        if let Some(el) =
            self.cached_states_by_block_n.iter().find_map(|s| s.state_diff.class_hashes.get(&contract_address))
        {
            return Ok(*el);
        }
        if let Some(read_cache) = &self.read_cache {
            if read_cache.is_contract_enabled(contract_address) {
                if let Some(value) = read_cache.get_class_hash(contract_address) {
                    exec_metrics().record_read_cache_hit(read_cache_kind::CLASS_HASH);
                    return Ok(value);
                }
                exec_metrics().record_read_cache_miss(read_cache_kind::CLASS_HASH);
            }
        }
        let value = self.inner.get_class_hash_at(contract_address)?;
        if let Some(read_cache) = &self.read_cache {
            read_cache.insert_class_hash_value(contract_address, value);
        }
        Ok(value)
    }
    fn get_compiled_class(&self, class_hash: ClassHash) -> StateResult<RunnableCompiledClass> {
        if let Some(el) = self.cached_states_by_block_n.iter().find_map(|s| s.classes.get(&class_hash)) {
            return <ApiContractClass as TryInto<RunnableCompiledClass>>::try_into(el.clone())
                .map_err(StateError::ProgramError);
        }
        self.inner.get_compiled_class(class_hash)
    }
    fn get_compiled_class_hash(&self, class_hash: ClassHash) -> StateResult<CompiledClassHash> {
        if let Some(el) =
            self.cached_states_by_block_n.iter().find_map(|s| s.state_diff.compiled_class_hashes.get(&class_hash))
        {
            return Ok(*el);
        }
        if let Some(read_cache) = &self.read_cache {
            if read_cache.should_cache_class_hash(class_hash) {
                if let Some(value) = read_cache.get_compiled_class_hash(class_hash) {
                    exec_metrics().record_read_cache_hit(read_cache_kind::COMPILED_CLASS_HASH);
                    return Ok(value);
                }
                exec_metrics().record_read_cache_miss(read_cache_kind::COMPILED_CLASS_HASH);
            }
        }
        let value = self.inner.get_compiled_class_hash(class_hash)?;
        if let Some(read_cache) = &self.read_cache {
            read_cache.insert_compiled_class_hash_value(class_hash, value);
        }
        Ok(value)
    }

    fn get_compiled_class_hash_v2(
        &self,
        class_hash: ClassHash,
        compiled_class: &RunnableCompiledClass,
    ) -> StateResult<CompiledClassHash> {
        if let Some(read_cache) = &self.read_cache {
            if read_cache.should_cache_class_hash(class_hash) {
                if let Some(value) = read_cache.get_compiled_class_hash_v2(class_hash) {
                    exec_metrics().record_read_cache_hit(read_cache_kind::COMPILED_CLASS_HASH_V2);
                    return Ok(value);
                }
                exec_metrics().record_read_cache_miss(read_cache_kind::COMPILED_CLASS_HASH_V2);
            }
        }
        let value = self.inner.get_compiled_class_hash_v2(class_hash, compiled_class)?;
        if let Some(read_cache) = &self.read_cache {
            read_cache.insert_compiled_class_hash_v2_value(class_hash, value);
        }
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::LayeredStateAdapter;
    use blockifier::state::{cached_state::StateMaps, state_api::StateReader};
    use mc_db::{ExecutionReadCacheConfig, MadaraBackend, MadaraBackendConfig};
    use mp_block::{
        header::{BlockTimestamp, GasPrices, PreconfirmedHeader},
        FullBlockWithoutCommitments,
    };
    use mp_chain_config::{ChainConfig, L1DataAvailabilityMode, StarknetVersion};
    use mp_convert::{Felt, ToFelt};
    use mp_state_update::{ContractStorageDiffItem, StateDiff, StorageEntry};
    use std::sync::Arc;

    fn insert_confirmed_block_with_storage(
        backend: &Arc<MadaraBackend>,
        block_number: u64,
        address: Felt,
        key: Felt,
        value: Felt,
    ) {
        backend
            .write_access()
            .add_full_block_with_classes(
                &FullBlockWithoutCommitments {
                    header: PreconfirmedHeader {
                        block_number,
                        sequencer_address: backend.chain_config().sequencer_address.to_felt(),
                        block_timestamp: BlockTimestamp::now(),
                        protocol_version: StarknetVersion::LATEST,
                        gas_prices: GasPrices::default(),
                        l1_da_mode: L1DataAvailabilityMode::Calldata,
                    },
                    state_diff: StateDiff {
                        storage_diffs: [ContractStorageDiffItem {
                            address,
                            storage_entries: vec![StorageEntry { key, value }],
                        }]
                        .into(),
                        ..Default::default()
                    },
                    transactions: vec![],
                    events: vec![],
                },
                /* classes */ &[],
                /* pre_v0_13_2_hash_override */ false,
            )
            .unwrap();
    }

    #[tokio::test]
    async fn test_layered_state_adapter() {
        let backend = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());
        backend.set_l1_gas_quote_for_testing();
        let mut adaptor = LayeredStateAdapter::new(backend.clone()).unwrap();

        // initial state (no genesis block)

        assert_eq!(adaptor.block_n(), 0);
        assert_eq!(adaptor.previous_block_n(), None);
        assert_eq!(adaptor.cached_states_by_block_n.len(), 0);

        assert_eq!(
            adaptor.get_storage_at(Felt::ONE.try_into().unwrap(), Felt::ONE.try_into().unwrap()).unwrap(),
            Felt::ZERO
        );
        assert_eq!(
            adaptor.get_storage_at(Felt::ONE.try_into().unwrap(), Felt::TWO.try_into().unwrap()).unwrap(),
            Felt::ZERO
        );
        assert_eq!(
            adaptor.get_storage_at(Felt::THREE.try_into().unwrap(), Felt::TWO.try_into().unwrap()).unwrap(),
            Felt::ZERO
        );

        // finish a block, not in db yet

        let mut state_maps = StateMaps::default();
        state_maps.storage.insert((Felt::ONE.try_into().unwrap(), Felt::ONE.try_into().unwrap()), Felt::THREE);
        adaptor.finish_block(state_maps, Default::default(), Default::default()).unwrap();

        assert_eq!(adaptor.block_n(), 1);
        assert_eq!(adaptor.previous_block_n(), Some(0));
        assert_eq!(adaptor.cached_states_by_block_n.len(), 1);

        assert_eq!(
            adaptor.get_storage_at(Felt::ONE.try_into().unwrap(), Felt::ONE.try_into().unwrap()).unwrap(),
            Felt::THREE
        ); // from cache
        assert_eq!(
            adaptor.get_storage_at(Felt::ONE.try_into().unwrap(), Felt::TWO.try_into().unwrap()).unwrap(),
            Felt::ZERO
        );
        assert_eq!(
            adaptor.get_storage_at(Felt::THREE.try_into().unwrap(), Felt::TWO.try_into().unwrap()).unwrap(),
            Felt::ZERO
        );

        // block is now in db

        insert_confirmed_block_with_storage(&backend, 0, Felt::ONE, Felt::ONE, Felt::THREE);

        assert_eq!(adaptor.block_n(), 1);
        assert_eq!(adaptor.previous_block_n(), Some(0));
        assert_eq!(adaptor.cached_states_by_block_n.len(), 1); // nothing changed here yet

        // finish another block, not in db yet but the earlier one is now in db. that one should have its state removed from the deque.

        let mut state_maps = StateMaps::default();
        state_maps.storage.insert((Felt::ONE.try_into().unwrap(), Felt::TWO.try_into().unwrap()), Felt::TWO);
        adaptor.finish_block(state_maps, Default::default(), Default::default()).unwrap();

        assert_eq!(adaptor.block_n(), 2);
        assert_eq!(adaptor.previous_block_n(), Some(1));
        assert_eq!(adaptor.cached_states_by_block_n.len(), 1);

        assert_eq!(
            adaptor.get_storage_at(Felt::ONE.try_into().unwrap(), Felt::ONE.try_into().unwrap()).unwrap(),
            Felt::THREE
        ); // from db
        assert_eq!(
            adaptor.get_storage_at(Felt::ONE.try_into().unwrap(), Felt::TWO.try_into().unwrap()).unwrap(),
            Felt::TWO
        ); // from cache
        assert_eq!(
            adaptor.get_storage_at(Felt::THREE.try_into().unwrap(), Felt::TWO.try_into().unwrap()).unwrap(),
            Felt::ZERO
        );
    }

    #[tokio::test]
    async fn test_layered_state_adapter_read_cache_updates_on_prune() {
        let config = MadaraBackendConfig {
            execution_read_cache: ExecutionReadCacheConfig {
                enabled: true,
                all_contracts: true,
                contracts: Vec::new(),
                max_memory_bytes: 1024 * 1024,
            },
            ..Default::default()
        };

        let backend = MadaraBackend::open_for_testing_with_config(ChainConfig::madara_test().into(), config);
        backend.set_l1_gas_quote_for_testing();
        let mut adaptor = LayeredStateAdapter::new(backend.clone()).unwrap();

        let contract_address = Felt::ONE.try_into().unwrap();
        let storage_key = Felt::ONE.try_into().unwrap();

        let mut state_maps = StateMaps::default();
        state_maps.storage.insert((contract_address, storage_key), Felt::THREE);
        adaptor.finish_block(state_maps, Default::default(), Default::default()).unwrap();

        {
            let cache = adaptor.read_cache.as_ref().unwrap();
            let guard = cache.inner.read().unwrap();
            assert_eq!(guard.storage.len(), 0);
        }

        insert_confirmed_block_with_storage(&backend, 0, Felt::ONE, Felt::ONE, Felt::THREE);

        adaptor.finish_block(StateMaps::default(), Default::default(), Default::default()).unwrap();

        {
            let cache = adaptor.read_cache.as_ref().unwrap();
            let guard = cache.inner.read().unwrap();
            assert_eq!(guard.storage.get(&(contract_address, storage_key)).unwrap().value, Felt::THREE);
        }
    }

    #[tokio::test]
    async fn test_layered_state_adapter_read_cache_contract_filter() {
        let allowed_address = Felt::ONE.try_into().unwrap();
        let config = MadaraBackendConfig {
            execution_read_cache: ExecutionReadCacheConfig {
                enabled: true,
                all_contracts: false,
                contracts: vec![allowed_address],
                max_memory_bytes: 1024 * 1024,
            },
            ..Default::default()
        };

        let backend = MadaraBackend::open_for_testing_with_config(ChainConfig::madara_test().into(), config);
        backend.set_l1_gas_quote_for_testing();
        let mut adaptor = LayeredStateAdapter::new(backend.clone()).unwrap();

        let blocked_address = Felt::TWO.try_into().unwrap();
        let storage_key = Felt::ONE.try_into().unwrap();

        let mut state_maps = StateMaps::default();
        state_maps.storage.insert((allowed_address, storage_key), Felt::THREE);
        state_maps.storage.insert((blocked_address, storage_key), Felt::TWO);
        adaptor.finish_block(state_maps, Default::default(), Default::default()).unwrap();

        insert_confirmed_block_with_storage(&backend, 0, Felt::ONE, Felt::ONE, Felt::THREE);

        adaptor.finish_block(StateMaps::default(), Default::default(), Default::default()).unwrap();

        let cache = adaptor.read_cache.as_ref().unwrap();
        let guard = cache.inner.read().unwrap();
        assert!(guard.storage.contains_key(&(allowed_address, storage_key)));
        assert!(!guard.storage.contains_key(&(blocked_address, storage_key)));
    }

    #[test]
    fn test_execution_read_cache_inner_trims_order_on_repeated_overwrite() {
        let mut inner = super::ExecutionReadCacheInner::default();
        let contract_address = Felt::ONE.try_into().unwrap();
        let storage_key = Felt::ONE.try_into().unwrap();

        for i in 0..256u64 {
            inner.insert_storage(contract_address, storage_key, Felt::from(i));
        }

        let max_bytes =
            super::ExecutionReadCacheInner::STORAGE_ENTRY_SIZE + super::ExecutionReadCacheInner::ORDER_ENTRY_SIZE;
        inner.evict_if_needed(max_bytes);

        assert!(inner.current_bytes <= max_bytes);
        assert_eq!(inner.storage.len(), 1);
        assert_eq!(inner.order.len(), 1);
    }
}
