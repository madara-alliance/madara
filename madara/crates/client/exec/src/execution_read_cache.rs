use crate::metrics::metrics as exec_metrics;
use blockifier::state::cached_state::StateMaps;
use mc_db::ExecutionReadCacheConfig;
use mp_convert::Felt;
use starknet_api::{
    core::{ClassHash, CompiledClassHash, ContractAddress, Nonce},
    state::StorageKey,
};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    mem::size_of,
    sync::RwLock,
};

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
pub(crate) struct ExecutionReadCache {
    all_contracts: bool,
    contracts: HashSet<ContractAddress>,
    max_bytes: usize,
    inner: RwLock<ExecutionReadCacheInner>,
}

impl ExecutionReadCache {
    fn with_read<R>(&self, f: impl FnOnce(&ExecutionReadCacheInner) -> R) -> R {
        let guard = self.inner.read().expect("Poisoned execution read cache lock");
        f(&guard)
    }

    fn with_write_recording<R>(&self, f: impl FnOnce(&mut ExecutionReadCacheInner) -> R) -> R {
        let mut guard = self.inner.write().expect("Poisoned execution read cache lock");
        let out = f(&mut guard);
        let bytes = guard.current_bytes as u64;
        drop(guard);
        exec_metrics().record_read_cache_size_bytes(bytes);
        out
    }

    pub(crate) fn from_config(config: &ExecutionReadCacheConfig) -> Option<Self> {
        if !config.enabled || config.max_memory_bytes == 0 {
            return None;
        }

        let all_contracts = config.contracts.is_none();
        let contracts = config.contracts.as_deref().unwrap_or_default().iter().copied().collect::<HashSet<_>>();

        if all_contracts {
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
            all_contracts,
            contracts,
            max_bytes: config.max_memory_bytes,
            inner: RwLock::new(ExecutionReadCacheInner::default()),
        })
    }

    pub(crate) fn is_contract_enabled(&self, contract_address: ContractAddress) -> bool {
        self.all_contracts || self.contracts.contains(&contract_address)
    }

    pub(crate) fn should_cache_class_hash(&self, class_hash: ClassHash) -> bool {
        if self.all_contracts {
            return true;
        }
        self.with_read(|guard| guard.cached_class_hashes.contains(&class_hash))
    }

    pub(crate) fn get_storage(&self, contract_address: ContractAddress, key: StorageKey) -> Option<Felt> {
        self.with_read(|guard| guard.storage.get(&(contract_address, key)).map(|entry| entry.value))
    }

    pub(crate) fn get_nonce(&self, contract_address: ContractAddress) -> Option<Nonce> {
        self.with_read(|guard| guard.nonces.get(&contract_address).map(|entry| entry.value))
    }

    pub(crate) fn get_class_hash(&self, contract_address: ContractAddress) -> Option<ClassHash> {
        self.with_read(|guard| guard.class_hashes.get(&contract_address).map(|entry| entry.value))
    }

    pub(crate) fn get_compiled_class_hash(&self, class_hash: ClassHash) -> Option<CompiledClassHash> {
        self.with_read(|guard| guard.compiled_class_hashes.get(&class_hash).map(|entry| entry.value))
    }

    pub(crate) fn get_compiled_class_hash_v2(&self, class_hash: ClassHash) -> Option<CompiledClassHash> {
        self.with_read(|guard| guard.compiled_class_hashes_v2.get(&class_hash).map(|entry| entry.value))
    }

    pub(crate) fn insert_storage_value(&self, contract_address: ContractAddress, key: StorageKey, value: Felt) {
        if !self.is_contract_enabled(contract_address) {
            return;
        }
        self.with_write_recording(|guard| guard.insert_storage(contract_address, key, value));
    }

    pub(crate) fn insert_nonce_value(&self, contract_address: ContractAddress, value: Nonce) {
        if !self.is_contract_enabled(contract_address) {
            return;
        }
        self.with_write_recording(|guard| guard.insert_nonce(contract_address, value));
    }

    pub(crate) fn insert_class_hash_value(&self, contract_address: ContractAddress, value: ClassHash) {
        if !self.is_contract_enabled(contract_address) {
            return;
        }
        self.with_write_recording(|guard| guard.insert_class_hash(contract_address, value));
    }

    pub(crate) fn insert_compiled_class_hash_value(&self, class_hash: ClassHash, value: CompiledClassHash) {
        if !self.should_cache_class_hash(class_hash) {
            return;
        }
        self.with_write_recording(|guard| guard.insert_compiled_class_hash(class_hash, value));
    }

    pub(crate) fn insert_compiled_class_hash_v2_value(&self, class_hash: ClassHash, value: CompiledClassHash) {
        if !self.should_cache_class_hash(class_hash) {
            return;
        }
        self.with_write_recording(|guard| guard.insert_compiled_class_hash_v2(class_hash, value));
    }

    pub(crate) fn apply_state_diff(&self, state_diff: &StateMaps) {
        self.with_write_recording(|guard| {
            for (contract_address, class_hash) in &state_diff.class_hashes {
                if self.is_contract_enabled(*contract_address) {
                    guard.insert_class_hash(*contract_address, *class_hash);
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
            } else {
                for (class_hash, compiled_hash) in &state_diff.compiled_class_hashes {
                    if guard.cached_class_hashes.contains(class_hash) {
                        guard.insert_compiled_class_hash(*class_hash, *compiled_hash);
                    }
                }
            }
        });
    }

    pub(crate) fn evict_if_needed(&self) {
        let evicted = self.with_write_recording(|guard| guard.evict_if_needed(self.max_bytes));
        if evicted > 0 {
            tracing::debug!("Execution read cache evicted {evicted} entries.");
        }
    }

    #[cfg(test)]
    pub(crate) fn test_storage_len(&self) -> usize {
        self.with_read(|guard| guard.storage.len())
    }
}

impl ExecutionReadCacheInner {
    const STORAGE_ENTRY_SIZE: usize = size_of::<CacheEntry<Felt>>() + size_of::<StorageEntryKey>();
    const NONCE_ENTRY_SIZE: usize = size_of::<CacheEntry<Nonce>>() + size_of::<ContractEntryKey>();
    const CLASS_HASH_ENTRY_SIZE: usize = size_of::<CacheEntry<ClassHash>>() + size_of::<ContractEntryKey>();
    // Approximate accounting for cached_class_hashes. This keeps max_memory_bytes meaningful without
    // tracking HashSet's internal bucket allocations precisely.
    const CACHED_CLASS_HASH_ENTRY_SIZE: usize = size_of::<ClassEntryKey>() + size_of::<usize>();
    const COMPILED_CLASS_HASH_ENTRY_SIZE: usize =
        size_of::<CacheEntry<CompiledClassHash>>() + size_of::<ClassEntryKey>();
    const ORDER_ENTRY_SIZE: usize = size_of::<CacheQueueEntry>();

    fn next_seq(&mut self) -> u64 {
        let seq = self.next_seq;
        self.next_seq = self.next_seq.wrapping_add(1);
        seq
    }

    fn insert_with_accounting(
        &mut self,
        entry_size: usize,
        queue_key: CacheKey,
        insert_or_replace: impl FnOnce(&mut Self, u64) -> bool,
    ) {
        let seq = self.next_seq();
        if insert_or_replace(self, seq) {
            self.current_bytes = self.current_bytes.saturating_sub(entry_size);
        }
        self.current_bytes = self.current_bytes.saturating_add(entry_size);
        self.order.push_back(CacheQueueEntry { key: queue_key, seq });
        self.current_bytes = self.current_bytes.saturating_add(Self::ORDER_ENTRY_SIZE);
    }

    fn insert_storage(&mut self, contract_address: ContractAddress, key: StorageKey, value: Felt) {
        let cache_key = (contract_address, key);
        self.insert_with_accounting(Self::STORAGE_ENTRY_SIZE, CacheKey::Storage(cache_key), |this, seq| {
            this.storage.insert(cache_key, CacheEntry { value, seq }).is_some()
        });
    }

    fn insert_nonce(&mut self, contract_address: ContractAddress, value: Nonce) {
        self.insert_with_accounting(Self::NONCE_ENTRY_SIZE, CacheKey::Nonce(contract_address), |this, seq| {
            this.nonces.insert(contract_address, CacheEntry { value, seq }).is_some()
        });
    }

    fn insert_class_hash(&mut self, contract_address: ContractAddress, value: ClassHash) {
        let seq = self.next_seq();
        if let Some(previous) = self.class_hashes.insert(contract_address, CacheEntry { value, seq }) {
            self.current_bytes = self.current_bytes.saturating_sub(Self::CLASS_HASH_ENTRY_SIZE);
            if self.cached_class_hashes.remove(&previous.value) {
                self.current_bytes = self.current_bytes.saturating_sub(Self::CACHED_CLASS_HASH_ENTRY_SIZE);
            }
        }
        self.current_bytes = self.current_bytes.saturating_add(Self::CLASS_HASH_ENTRY_SIZE);
        if self.cached_class_hashes.insert(value) {
            self.current_bytes = self.current_bytes.saturating_add(Self::CACHED_CLASS_HASH_ENTRY_SIZE);
        }
        self.order.push_back(CacheQueueEntry { key: CacheKey::ClassHash(contract_address), seq });
        self.current_bytes = self.current_bytes.saturating_add(Self::ORDER_ENTRY_SIZE);
    }

    fn insert_compiled_class_hash(&mut self, class_hash: ClassHash, value: CompiledClassHash) {
        self.insert_with_accounting(
            Self::COMPILED_CLASS_HASH_ENTRY_SIZE,
            CacheKey::CompiledClassHash(class_hash),
            |this, seq| this.compiled_class_hashes.insert(class_hash, CacheEntry { value, seq }).is_some(),
        );
    }

    fn insert_compiled_class_hash_v2(&mut self, class_hash: ClassHash, value: CompiledClassHash) {
        self.insert_with_accounting(
            Self::COMPILED_CLASS_HASH_ENTRY_SIZE,
            CacheKey::CompiledClassHashV2(class_hash),
            |this, seq| this.compiled_class_hashes_v2.insert(class_hash, CacheEntry { value, seq }).is_some(),
        );
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
                        let existing = self.class_hashes.remove(&contract_address).expect("entry exists");
                        self.current_bytes = self.current_bytes.saturating_sub(Self::CLASS_HASH_ENTRY_SIZE);
                        if self.cached_class_hashes.remove(&existing.value) {
                            self.current_bytes = self.current_bytes.saturating_sub(Self::CACHED_CLASS_HASH_ENTRY_SIZE);
                        }
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

#[cfg(test)]
mod tests {
    use super::{ExecutionReadCache, ExecutionReadCacheInner};
    use mc_db::ExecutionReadCacheConfig;
    use mp_convert::Felt;
    use starknet_api::core::{ClassHash, CompiledClassHash, Nonce};

    #[test]
    fn test_execution_read_cache_inner_trims_order_on_repeated_overwrite() {
        let mut inner = ExecutionReadCacheInner::default();
        let contract_address = Felt::ONE.try_into().unwrap();
        let storage_key = Felt::ONE.try_into().unwrap();

        for i in 0..256u64 {
            inner.insert_storage(contract_address, storage_key, Felt::from(i));
        }

        let max_bytes = ExecutionReadCacheInner::STORAGE_ENTRY_SIZE + ExecutionReadCacheInner::ORDER_ENTRY_SIZE;
        inner.evict_if_needed(max_bytes);

        assert!(inner.current_bytes <= max_bytes);
        assert_eq!(inner.storage.len(), 1);
        assert_eq!(inner.order.len(), 1);
    }

    #[test]
    fn test_execution_read_cache_allowlist_supports_nonce_and_class_hash_paths() {
        let allowed_address = Felt::ONE.try_into().unwrap();
        let blocked_address = Felt::TWO.try_into().unwrap();
        let allowed_class_hash = ClassHash(Felt::from(100u64));
        let blocked_class_hash = ClassHash(Felt::from(200u64));
        let compiled_hash = CompiledClassHash(Felt::from(300u64));
        let compiled_hash_v2 = CompiledClassHash(Felt::from(400u64));

        let config = ExecutionReadCacheConfig {
            enabled: true,
            contracts: Some(vec![allowed_address]),
            max_memory_bytes: 1024 * 1024,
        };
        let cache = ExecutionReadCache::from_config(&config).unwrap();

        cache.insert_nonce_value(allowed_address, Nonce(Felt::from(1u64)));
        cache.insert_nonce_value(blocked_address, Nonce(Felt::from(2u64)));
        assert_eq!(cache.get_nonce(allowed_address), Some(Nonce(Felt::from(1u64))));
        assert_eq!(cache.get_nonce(blocked_address), None);

        cache.insert_class_hash_value(allowed_address, allowed_class_hash);
        cache.insert_class_hash_value(blocked_address, blocked_class_hash);
        assert_eq!(cache.get_class_hash(allowed_address), Some(allowed_class_hash));
        assert_eq!(cache.get_class_hash(blocked_address), None);
        assert!(cache.should_cache_class_hash(allowed_class_hash));
        assert!(!cache.should_cache_class_hash(blocked_class_hash));

        cache.insert_compiled_class_hash_value(allowed_class_hash, compiled_hash);
        cache.insert_compiled_class_hash_v2_value(allowed_class_hash, compiled_hash_v2);
        assert_eq!(cache.get_compiled_class_hash(allowed_class_hash), Some(compiled_hash));
        assert_eq!(cache.get_compiled_class_hash_v2(allowed_class_hash), Some(compiled_hash_v2));

        cache.insert_compiled_class_hash_value(blocked_class_hash, CompiledClassHash(Felt::from(301u64)));
        cache.insert_compiled_class_hash_v2_value(blocked_class_hash, CompiledClassHash(Felt::from(401u64)));
        assert_eq!(cache.get_compiled_class_hash(blocked_class_hash), None);
        assert_eq!(cache.get_compiled_class_hash_v2(blocked_class_hash), None);
    }

    #[test]
    fn test_execution_read_cache_all_contracts_supports_compiled_hash_paths() {
        let address = Felt::ONE.try_into().unwrap();
        let class_hash = ClassHash(Felt::from(500u64));
        let class_hash_without_contract_mapping = ClassHash(Felt::from(600u64));
        let compiled_hash = CompiledClassHash(Felt::from(700u64));
        let compiled_hash_v2 = CompiledClassHash(Felt::from(800u64));

        let config = ExecutionReadCacheConfig { enabled: true, contracts: None, max_memory_bytes: 1024 * 1024 };
        let cache = ExecutionReadCache::from_config(&config).unwrap();

        cache.insert_nonce_value(address, Nonce(Felt::from(3u64)));
        cache.insert_class_hash_value(address, class_hash);
        assert_eq!(cache.get_nonce(address), Some(Nonce(Felt::from(3u64))));
        assert_eq!(cache.get_class_hash(address), Some(class_hash));

        cache.insert_compiled_class_hash_value(class_hash_without_contract_mapping, compiled_hash);
        cache.insert_compiled_class_hash_v2_value(class_hash_without_contract_mapping, compiled_hash_v2);
        assert!(cache.should_cache_class_hash(class_hash_without_contract_mapping));
        assert_eq!(cache.get_compiled_class_hash(class_hash_without_contract_mapping), Some(compiled_hash));
        assert_eq!(cache.get_compiled_class_hash_v2(class_hash_without_contract_mapping), Some(compiled_hash_v2));
    }

    #[test]
    fn test_cached_class_hashes_removed_when_contract_class_hash_entry_evicted() {
        let allowed_address = Felt::ONE.try_into().unwrap();
        let class_hash = ClassHash(Felt::from(123u64));
        let compiled_hash = CompiledClassHash(Felt::from(456u64));

        let config =
            ExecutionReadCacheConfig { enabled: true, contracts: Some(vec![allowed_address]), max_memory_bytes: 1 };
        let cache = ExecutionReadCache::from_config(&config).unwrap();

        cache.insert_class_hash_value(allowed_address, class_hash);
        assert!(cache.should_cache_class_hash(class_hash));

        // Evict everything; this will evict the ClassHash(contract) entry and should remove the class hash from the set.
        cache.evict_if_needed();
        assert!(!cache.should_cache_class_hash(class_hash));

        // Even when class isn't considered cacheable anymore, fallback/insert path should not error.
        cache.insert_compiled_class_hash_value(class_hash, compiled_hash);
        assert_eq!(cache.get_compiled_class_hash(class_hash), None);
    }
}
