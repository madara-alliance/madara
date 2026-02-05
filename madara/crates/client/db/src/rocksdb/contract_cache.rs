//! Contract-specific caching layer for reducing RocksDB I/O.
//!
//! This module provides an in-memory cache for frequently accessed contract state,
//! sitting between the RocksDB storage layer and consumers. The cache:
//!
//! - Only caches specified contract addresses (explicit configuration)
//! - Automatically caches classes used by those contracts
//! - Has configurable memory limits with LRU eviction
//! - Stays consistent with DB (updates on writes, invalidates on reorgs)
//! - Is ephemeral (no persistence, rebuilt on restart)
//!
//! ## Architecture
//!
//! ```text
//! RocksDBStorage → ContractCache → RocksDBStorageInner → RocksDB
//! ```
//!
//! The cache intercepts reads/writes inside `RocksDBStorage`, transparent to all consumers.
//!
//! ## Cache Validity
//!
//! Each cache entry stores the block number when the value was written. An entry is valid
//! for queries at `block_n >= entry.block_n`. Historical queries (`block_n < entry.block_n`)
//! bypass the cache and hit RocksDB directly.

use crate::storage::{ClassInfoWithBlockN, CompiledSierraWithBlockN};
use dashmap::DashMap;
use mp_class::ConvertedClass;
use mp_convert::Felt;
use mp_state_update::StateDiff;
use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Instant;

/// Configuration for contract-specific caching.
#[derive(Debug, Clone)]
pub struct ContractCacheConfig {
    /// Whether the cache is enabled.
    pub enabled: bool,
    /// Contract addresses to cache. Only these contracts will have their state cached.
    pub cached_contract_addresses: HashSet<Felt>,
    /// Maximum memory budget for the cache in bytes.
    pub max_memory_bytes: usize,
}

impl Default for ContractCacheConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            cached_contract_addresses: HashSet::new(),
            max_memory_bytes: 256 * 1024 * 1024, // 256 MiB default
        }
    }
}

/// A cached storage entry for contract storage values.
#[derive(Debug, Clone)]
struct CachedStorageEntry {
    /// The storage value.
    value: Felt,
    /// Block number when this value was written.
    block_n: u64,
    /// Last access time for LRU eviction.
    last_access: Instant,
}

/// A cached nonce entry for contract nonces.
#[derive(Debug, Clone)]
struct CachedNonceEntry {
    /// The nonce value.
    nonce: Felt,
    /// Block number when this nonce was written.
    block_n: u64,
    /// Last access time for LRU eviction.
    last_access: Instant,
}

/// A cached class hash entry for contract class hashes.
#[derive(Debug, Clone)]
struct CachedClassHashEntry {
    /// The class hash.
    class_hash: Felt,
    /// Block number when this class hash was set (deployment or replacement).
    block_n: u64,
    /// Last access time for LRU eviction.
    last_access: Instant,
}

/// A cached class info entry.
#[derive(Debug, Clone)]
struct CachedClassInfoEntry {
    /// The class info with block number.
    info: ClassInfoWithBlockN,
    /// Last access time for LRU eviction.
    last_access: Instant,
}

/// A cached compiled class entry.
#[derive(Debug, Clone)]
struct CachedCompiledEntry {
    /// The compiled class with block number.
    compiled: CompiledSierraWithBlockN,
    /// Last access time for LRU eviction.
    last_access: Instant,
}

/// Estimated size in bytes for different entry types.
const STORAGE_ENTRY_SIZE: usize = 120; // keys + value + metadata
const NONCE_ENTRY_SIZE: usize = 80;
const CLASS_HASH_ENTRY_SIZE: usize = 80;
const CLASS_INFO_ENTRY_BASE_SIZE: usize = 2048; // Variable, this is a base estimate
const COMPILED_CLASS_ENTRY_BASE_SIZE: usize = 65536; // Variable, typically 64-256 KB

/// Contract-specific cache for reducing RocksDB I/O.
///
/// This cache stores state for explicitly configured contract addresses and automatically
/// caches any classes used by those contracts.
pub struct ContractCache {
    config: ContractCacheConfig,

    // Contract state caches (only for configured addresses)
    /// (contract_address, storage_key) -> value
    storage_cache: DashMap<(Felt, Felt), CachedStorageEntry>,
    /// contract_address -> nonce
    nonce_cache: DashMap<Felt, CachedNonceEntry>,
    /// contract_address -> class_hash
    class_hash_cache: DashMap<Felt, CachedClassHashEntry>,

    // Class caches (auto-populated when accessed via cached contracts)
    /// Set of class_hashes that should be cached (belonging to cached contracts)
    cached_class_hashes: DashMap<Felt, ()>,
    /// class_hash -> class_info
    class_info_cache: DashMap<Felt, CachedClassInfoEntry>,
    /// compiled_class_hash -> compiled_class
    compiled_class_cache: DashMap<Felt, CachedCompiledEntry>,

    // Memory tracking
    current_memory_bytes: AtomicUsize,

    // Metrics
    hits: AtomicU64,
    misses: AtomicU64,
    evictions: AtomicU64,
}

impl std::fmt::Debug for ContractCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContractCache")
            .field("config", &self.config)
            .field("storage_entries", &self.storage_cache.len())
            .field("nonce_entries", &self.nonce_cache.len())
            .field("class_hash_entries", &self.class_hash_cache.len())
            .field("class_info_entries", &self.class_info_cache.len())
            .field("compiled_class_entries", &self.compiled_class_cache.len())
            .field("current_memory_bytes", &self.current_memory_bytes.load(Ordering::Relaxed))
            .field("hits", &self.hits.load(Ordering::Relaxed))
            .field("misses", &self.misses.load(Ordering::Relaxed))
            .field("evictions", &self.evictions.load(Ordering::Relaxed))
            .finish()
    }
}

impl ContractCache {
    /// Create a new contract cache with the given configuration.
    pub fn new(config: ContractCacheConfig) -> Self {
        tracing::info!(
            "Creating contract cache: enabled={}, contracts={}, max_memory={}MiB",
            config.enabled,
            config.cached_contract_addresses.len(),
            config.max_memory_bytes / (1024 * 1024)
        );

        Self {
            config,
            storage_cache: DashMap::new(),
            nonce_cache: DashMap::new(),
            class_hash_cache: DashMap::new(),
            cached_class_hashes: DashMap::new(),
            class_info_cache: DashMap::new(),
            compiled_class_cache: DashMap::new(),
            current_memory_bytes: AtomicUsize::new(0),
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
            evictions: AtomicU64::new(0),
        }
    }

    /// Check if the cache is enabled.
    #[inline]
    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    /// Check if a contract address should be cached.
    #[inline]
    pub fn should_cache_contract(&self, contract_address: &Felt) -> bool {
        self.config.enabled && self.config.cached_contract_addresses.contains(contract_address)
    }

    /// Check if a class hash should be cached.
    #[inline]
    pub fn should_cache_class(&self, class_hash: &Felt) -> bool {
        self.config.enabled && self.cached_class_hashes.contains_key(class_hash)
    }

    // =========================================================================
    // STORAGE CACHE OPERATIONS
    // =========================================================================

    /// Get a storage value from the cache.
    ///
    /// Returns `Some(value)` if the entry is cached and valid for the given block_n.
    /// Returns `None` on cache miss or if the query is for a historical block before
    /// the cached value was written.
    pub fn get_storage(&self, block_n: u64, contract_address: &Felt, key: &Felt) -> Option<Felt> {
        if !self.should_cache_contract(contract_address) {
            return None;
        }

        let cache_key = (*contract_address, *key);
        if let Some(mut entry) = self.storage_cache.get_mut(&cache_key) {
            // Entry is valid if query_block_n >= entry.block_n
            if block_n >= entry.block_n {
                entry.last_access = Instant::now();
                self.hits.fetch_add(1, Ordering::Relaxed);
                return Some(entry.value);
            }
            // Historical query - need to hit DB
        }

        self.misses.fetch_add(1, Ordering::Relaxed);
        None
    }

    /// Put a storage value into the cache.
    ///
    /// Only caches if the contract is in the configured set.
    pub fn put_storage(&self, block_n: u64, contract_address: &Felt, key: &Felt, value: Felt) {
        if !self.should_cache_contract(contract_address) {
            return;
        }

        let cache_key = (*contract_address, *key);
        let is_new = !self.storage_cache.contains_key(&cache_key);

        self.storage_cache.insert(cache_key, CachedStorageEntry { value, block_n, last_access: Instant::now() });

        if is_new {
            self.current_memory_bytes.fetch_add(STORAGE_ENTRY_SIZE, Ordering::Relaxed);
            self.evict_if_needed();
        }
    }

    // =========================================================================
    // NONCE CACHE OPERATIONS
    // =========================================================================

    /// Get a nonce from the cache.
    pub fn get_nonce(&self, block_n: u64, contract_address: &Felt) -> Option<Felt> {
        if !self.should_cache_contract(contract_address) {
            return None;
        }

        if let Some(mut entry) = self.nonce_cache.get_mut(contract_address) {
            if block_n >= entry.block_n {
                entry.last_access = Instant::now();
                self.hits.fetch_add(1, Ordering::Relaxed);
                return Some(entry.nonce);
            }
        }

        self.misses.fetch_add(1, Ordering::Relaxed);
        None
    }

    /// Put a nonce into the cache.
    pub fn put_nonce(&self, block_n: u64, contract_address: &Felt, nonce: Felt) {
        if !self.should_cache_contract(contract_address) {
            return;
        }

        let is_new = !self.nonce_cache.contains_key(contract_address);

        self.nonce_cache.insert(*contract_address, CachedNonceEntry { nonce, block_n, last_access: Instant::now() });

        if is_new {
            self.current_memory_bytes.fetch_add(NONCE_ENTRY_SIZE, Ordering::Relaxed);
            self.evict_if_needed();
        }
    }

    // =========================================================================
    // CLASS HASH CACHE OPERATIONS
    // =========================================================================

    /// Get a class hash from the cache.
    ///
    /// If found, also registers the class_hash for automatic class caching.
    pub fn get_class_hash(&self, block_n: u64, contract_address: &Felt) -> Option<Felt> {
        if !self.should_cache_contract(contract_address) {
            return None;
        }

        if let Some(mut entry) = self.class_hash_cache.get_mut(contract_address) {
            if block_n >= entry.block_n {
                entry.last_access = Instant::now();
                self.hits.fetch_add(1, Ordering::Relaxed);

                // Auto-register this class_hash for caching
                self.cached_class_hashes.insert(entry.class_hash, ());

                return Some(entry.class_hash);
            }
        }

        self.misses.fetch_add(1, Ordering::Relaxed);
        None
    }

    /// Put a class hash into the cache.
    ///
    /// Also registers the class_hash for automatic class caching.
    pub fn put_class_hash(&self, block_n: u64, contract_address: &Felt, class_hash: Felt) {
        if !self.should_cache_contract(contract_address) {
            return;
        }

        let is_new = !self.class_hash_cache.contains_key(contract_address);

        self.class_hash_cache
            .insert(*contract_address, CachedClassHashEntry { class_hash, block_n, last_access: Instant::now() });

        // Auto-register this class_hash for caching
        self.cached_class_hashes.insert(class_hash, ());

        if is_new {
            self.current_memory_bytes.fetch_add(CLASS_HASH_ENTRY_SIZE, Ordering::Relaxed);
            self.evict_if_needed();
        }
    }

    // =========================================================================
    // CLASS INFO CACHE OPERATIONS
    // =========================================================================

    /// Get class info from the cache.
    pub fn get_class_info(&self, class_hash: &Felt) -> Option<ClassInfoWithBlockN> {
        if !self.should_cache_class(class_hash) {
            return None;
        }

        if let Some(mut entry) = self.class_info_cache.get_mut(class_hash) {
            entry.last_access = Instant::now();
            self.hits.fetch_add(1, Ordering::Relaxed);
            return Some(entry.info.clone());
        }

        self.misses.fetch_add(1, Ordering::Relaxed);
        None
    }

    /// Put class info into the cache.
    pub fn put_class_info(&self, class_hash: &Felt, info: ClassInfoWithBlockN) {
        if !self.should_cache_class(class_hash) {
            return;
        }

        let is_new = !self.class_info_cache.contains_key(class_hash);

        self.class_info_cache.insert(*class_hash, CachedClassInfoEntry { info, last_access: Instant::now() });

        if is_new {
            self.current_memory_bytes.fetch_add(CLASS_INFO_ENTRY_BASE_SIZE, Ordering::Relaxed);
            self.evict_if_needed();
        }
    }

    // =========================================================================
    // COMPILED CLASS CACHE OPERATIONS
    // =========================================================================

    /// Get compiled class from the cache.
    pub fn get_compiled_class(&self, compiled_class_hash: &Felt) -> Option<CompiledSierraWithBlockN> {
        // For compiled classes, we check if any cached class uses this hash
        // This is a simplification - in practice, we'd need a reverse mapping
        if !self.config.enabled {
            return None;
        }

        if let Some(mut entry) = self.compiled_class_cache.get_mut(compiled_class_hash) {
            entry.last_access = Instant::now();
            self.hits.fetch_add(1, Ordering::Relaxed);
            return Some(entry.compiled.clone());
        }

        self.misses.fetch_add(1, Ordering::Relaxed);
        None
    }

    /// Put compiled class into the cache.
    pub fn put_compiled_class(&self, compiled_class_hash: &Felt, compiled: CompiledSierraWithBlockN) {
        if !self.config.enabled {
            return;
        }

        let is_new = !self.compiled_class_cache.contains_key(compiled_class_hash);

        self.compiled_class_cache
            .insert(*compiled_class_hash, CachedCompiledEntry { compiled, last_access: Instant::now() });

        if is_new {
            self.current_memory_bytes.fetch_add(COMPILED_CLASS_ENTRY_BASE_SIZE, Ordering::Relaxed);
            self.evict_if_needed();
        }
    }

    // =========================================================================
    // STATE DIFF APPLICATION
    // =========================================================================

    /// Apply a state diff to update the cache after a successful DB write.
    ///
    /// This should be called after `write_state_diff` completes successfully.
    pub fn apply_state_diff(&self, block_n: u64, state_diff: &StateDiff) {
        if !self.config.enabled {
            return;
        }

        // Update storage values
        for storage_diff in &state_diff.storage_diffs {
            if self.should_cache_contract(&storage_diff.address) {
                for entry in &storage_diff.storage_entries {
                    self.storage_cache.insert(
                        (storage_diff.address, entry.key),
                        CachedStorageEntry { value: entry.value, block_n, last_access: Instant::now() },
                    );
                }
            }
        }

        // Update nonces
        for nonce_update in &state_diff.nonces {
            if self.should_cache_contract(&nonce_update.contract_address) {
                self.nonce_cache.insert(
                    nonce_update.contract_address,
                    CachedNonceEntry { nonce: nonce_update.nonce, block_n, last_access: Instant::now() },
                );
            }
        }

        // Update class hashes for deployed contracts
        for deployed in &state_diff.deployed_contracts {
            if self.should_cache_contract(&deployed.address) {
                self.class_hash_cache.insert(
                    deployed.address,
                    CachedClassHashEntry { class_hash: deployed.class_hash, block_n, last_access: Instant::now() },
                );
                // Register the class hash for caching
                self.cached_class_hashes.insert(deployed.class_hash, ());
            }
        }

        // Update class hashes for replaced classes
        for replaced in &state_diff.replaced_classes {
            if self.should_cache_contract(&replaced.contract_address) {
                self.class_hash_cache.insert(
                    replaced.contract_address,
                    CachedClassHashEntry { class_hash: replaced.class_hash, block_n, last_access: Instant::now() },
                );
                // Register the class hash for caching
                self.cached_class_hashes.insert(replaced.class_hash, ());
            }
        }
    }

    /// Apply classes to update the cache after a successful DB write.
    ///
    /// This should be called after `write_classes` completes successfully.
    pub fn apply_classes(&self, block_n: u64, converted_classes: &[ConvertedClass]) {
        if !self.config.enabled {
            return;
        }

        for converted_class in converted_classes {
            let class_hash = *converted_class.class_hash();

            // Only cache classes that belong to cached contracts
            if !self.should_cache_class(&class_hash) {
                continue;
            }

            // Cache class info
            self.class_info_cache.insert(
                class_hash,
                CachedClassInfoEntry {
                    info: ClassInfoWithBlockN { block_number: block_n, class_info: converted_class.info() },
                    last_access: Instant::now(),
                },
            );

            // Cache compiled class for Sierra classes
            if let ConvertedClass::Sierra(sierra) = converted_class {
                // Use canonical compiled_class_hash (v2 if present, else v1)
                if let Some(canonical_hash) = sierra.info.compiled_class_hash_v2.or(sierra.info.compiled_class_hash) {
                    self.compiled_class_cache.insert(
                        canonical_hash,
                        CachedCompiledEntry {
                            compiled: CompiledSierraWithBlockN {
                                block_number: block_n,
                                compiled_sierra: sierra.compiled.clone(),
                            },
                            last_access: Instant::now(),
                        },
                    );
                }
            }
        }
    }

    // =========================================================================
    // INVALIDATION
    // =========================================================================

    /// Invalidate all cache entries from a given block number onwards.
    ///
    /// This should be called during reorg to ensure cache consistency.
    /// Entries with `block_n >= from_block_n` are removed.
    pub fn invalidate_from_block(&self, from_block_n: u64) {
        if !self.config.enabled {
            return;
        }

        tracing::info!("Invalidating contract cache from block {}", from_block_n);

        let mut invalidated = 0u64;

        // Invalidate storage entries
        self.storage_cache.retain(|_, entry| {
            if entry.block_n >= from_block_n {
                invalidated += 1;
                false
            } else {
                true
            }
        });

        // Invalidate nonce entries
        self.nonce_cache.retain(|_, entry| {
            if entry.block_n >= from_block_n {
                invalidated += 1;
                false
            } else {
                true
            }
        });

        // Invalidate class hash entries
        self.class_hash_cache.retain(|_, entry| {
            if entry.block_n >= from_block_n {
                invalidated += 1;
                false
            } else {
                true
            }
        });

        // Invalidate class info entries
        self.class_info_cache.retain(|_, entry| {
            if entry.info.block_number >= from_block_n {
                invalidated += 1;
                false
            } else {
                true
            }
        });

        // Invalidate compiled class entries
        self.compiled_class_cache.retain(|_, entry| {
            if entry.compiled.block_number >= from_block_n {
                invalidated += 1;
                false
            } else {
                true
            }
        });

        // Recalculate memory usage (rough estimate)
        let new_memory = self.storage_cache.len() * STORAGE_ENTRY_SIZE
            + self.nonce_cache.len() * NONCE_ENTRY_SIZE
            + self.class_hash_cache.len() * CLASS_HASH_ENTRY_SIZE
            + self.class_info_cache.len() * CLASS_INFO_ENTRY_BASE_SIZE
            + self.compiled_class_cache.len() * COMPILED_CLASS_ENTRY_BASE_SIZE;
        self.current_memory_bytes.store(new_memory, Ordering::Relaxed);

        tracing::info!("Invalidated {} cache entries from block {}", invalidated, from_block_n);
    }

    // =========================================================================
    // MEMORY MANAGEMENT
    // =========================================================================

    /// Check if eviction is needed and perform LRU eviction if so.
    fn evict_if_needed(&self) {
        let current = self.current_memory_bytes.load(Ordering::Relaxed);
        if current <= self.config.max_memory_bytes {
            return;
        }

        // We need to evict some entries
        let target = self.config.max_memory_bytes * 90 / 100; // Evict to 90% capacity

        tracing::debug!(
            "Contract cache eviction triggered: current={}MiB, max={}MiB, target={}MiB",
            current / (1024 * 1024),
            self.config.max_memory_bytes / (1024 * 1024),
            target / (1024 * 1024)
        );

        self.evict_lru_until(target);
    }

    /// Evict LRU entries until memory usage is at or below the target.
    fn evict_lru_until(&self, target_bytes: usize) {
        // Collect all entries with their last access times
        let mut storage_entries: Vec<_> =
            self.storage_cache.iter().map(|entry| (*entry.key(), entry.last_access)).collect();
        let mut nonce_entries: Vec<_> =
            self.nonce_cache.iter().map(|entry| (*entry.key(), entry.last_access)).collect();

        // Sort by last access (oldest first)
        storage_entries.sort_by_key(|(_, t)| *t);
        nonce_entries.sort_by_key(|(_, t)| *t);

        let mut evicted = 0u64;
        let mut current = self.current_memory_bytes.load(Ordering::Relaxed);

        // Evict storage entries first (most numerous)
        for (key, _) in storage_entries {
            if current <= target_bytes {
                break;
            }
            if self.storage_cache.remove(&key).is_some() {
                current = current.saturating_sub(STORAGE_ENTRY_SIZE);
                evicted += 1;
            }
        }

        // Evict nonce entries if needed
        for (key, _) in nonce_entries {
            if current <= target_bytes {
                break;
            }
            if self.nonce_cache.remove(&key).is_some() {
                current = current.saturating_sub(NONCE_ENTRY_SIZE);
                evicted += 1;
            }
        }

        self.current_memory_bytes.store(current, Ordering::Relaxed);
        self.evictions.fetch_add(evicted, Ordering::Relaxed);

        tracing::debug!("Evicted {} entries, new memory usage: {}MiB", evicted, current / (1024 * 1024));
    }

    // =========================================================================
    // METRICS
    // =========================================================================

    /// Get the number of cache hits.
    pub fn hits(&self) -> u64 {
        self.hits.load(Ordering::Relaxed)
    }

    /// Get the number of cache misses.
    pub fn misses(&self) -> u64 {
        self.misses.load(Ordering::Relaxed)
    }

    /// Get the number of evictions.
    pub fn evictions(&self) -> u64 {
        self.evictions.load(Ordering::Relaxed)
    }

    /// Get the current memory usage in bytes.
    pub fn memory_bytes(&self) -> usize {
        self.current_memory_bytes.load(Ordering::Relaxed)
    }

    /// Get the number of storage entries.
    pub fn storage_entry_count(&self) -> usize {
        self.storage_cache.len()
    }

    /// Get the number of nonce entries.
    pub fn nonce_entry_count(&self) -> usize {
        self.nonce_cache.len()
    }

    /// Get the number of class hash entries.
    pub fn class_hash_entry_count(&self) -> usize {
        self.class_hash_cache.len()
    }

    /// Get the number of class info entries.
    pub fn class_info_entry_count(&self) -> usize {
        self.class_info_cache.len()
    }

    /// Get the number of compiled class entries.
    pub fn compiled_class_entry_count(&self) -> usize {
        self.compiled_class_cache.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mp_state_update::{ContractStorageDiffItem, DeployedContractItem, NonceUpdate, StorageEntry};

    fn make_felt(v: u64) -> Felt {
        Felt::from(v)
    }

    fn make_config_with_contracts(contracts: Vec<Felt>) -> ContractCacheConfig {
        ContractCacheConfig {
            enabled: true,
            cached_contract_addresses: contracts.into_iter().collect(),
            max_memory_bytes: 1024 * 1024, // 1 MiB for tests
        }
    }

    #[test]
    fn test_disabled_cache_returns_none() {
        let config = ContractCacheConfig::default();
        assert!(!config.enabled);

        let cache = ContractCache::new(config);
        let addr = make_felt(1);
        let key = make_felt(2);

        // Put should be a no-op
        cache.put_storage(0, &addr, &key, make_felt(100));

        // Get should return None
        assert!(cache.get_storage(0, &addr, &key).is_none());
    }

    #[test]
    fn test_uncached_contract_returns_none() {
        let cached_addr = make_felt(1);
        let uncached_addr = make_felt(2);
        let config = make_config_with_contracts(vec![cached_addr]);

        let cache = ContractCache::new(config);
        let key = make_felt(10);

        // Put to uncached contract should be a no-op
        cache.put_storage(0, &uncached_addr, &key, make_felt(100));

        // Get from uncached contract should return None
        assert!(cache.get_storage(0, &uncached_addr, &key).is_none());
    }

    #[test]
    fn test_storage_cache_hit() {
        let addr = make_felt(1);
        let config = make_config_with_contracts(vec![addr]);
        let cache = ContractCache::new(config);

        let key = make_felt(10);
        let value = make_felt(100);

        // Put at block 5
        cache.put_storage(5, &addr, &key, value);

        // Get at block 5 should hit
        assert_eq!(cache.get_storage(5, &addr, &key), Some(value));

        // Get at block 10 should also hit (query_block >= cached_block)
        assert_eq!(cache.get_storage(10, &addr, &key), Some(value));

        assert_eq!(cache.hits(), 2);
        assert_eq!(cache.misses(), 0);
    }

    #[test]
    fn test_storage_cache_historical_miss() {
        let addr = make_felt(1);
        let config = make_config_with_contracts(vec![addr]);
        let cache = ContractCache::new(config);

        let key = make_felt(10);
        let value = make_felt(100);

        // Put at block 5
        cache.put_storage(5, &addr, &key, value);

        // Get at block 3 should miss (historical query before cached value)
        assert!(cache.get_storage(3, &addr, &key).is_none());

        assert_eq!(cache.hits(), 0);
        assert_eq!(cache.misses(), 1);
    }

    #[test]
    fn test_nonce_cache() {
        let addr = make_felt(1);
        let config = make_config_with_contracts(vec![addr]);
        let cache = ContractCache::new(config);

        let nonce = make_felt(42);

        cache.put_nonce(5, &addr, nonce);
        assert_eq!(cache.get_nonce(5, &addr), Some(nonce));
        assert_eq!(cache.get_nonce(10, &addr), Some(nonce));
        assert!(cache.get_nonce(3, &addr).is_none()); // Historical miss
    }

    #[test]
    fn test_class_hash_cache_auto_registers_class() {
        let addr = make_felt(1);
        let config = make_config_with_contracts(vec![addr]);
        let cache = ContractCache::new(config);

        let class_hash = make_felt(999);

        // Initially, class should not be cached
        assert!(!cache.should_cache_class(&class_hash));

        // Put class hash
        cache.put_class_hash(5, &addr, class_hash);

        // Now class should be auto-registered for caching
        assert!(cache.should_cache_class(&class_hash));

        // Get should work
        assert_eq!(cache.get_class_hash(5, &addr), Some(class_hash));
    }

    #[test]
    fn test_apply_state_diff() {
        let addr = make_felt(1);
        let config = make_config_with_contracts(vec![addr]);
        let cache = ContractCache::new(config);

        let state_diff = StateDiff {
            storage_diffs: vec![ContractStorageDiffItem {
                address: addr,
                storage_entries: vec![
                    StorageEntry { key: make_felt(10), value: make_felt(100) },
                    StorageEntry { key: make_felt(20), value: make_felt(200) },
                ],
            }],
            nonces: vec![NonceUpdate { contract_address: addr, nonce: make_felt(5) }],
            deployed_contracts: vec![DeployedContractItem { address: addr, class_hash: make_felt(999) }],
            ..Default::default()
        };

        cache.apply_state_diff(10, &state_diff);

        // Storage should be cached
        assert_eq!(cache.get_storage(10, &addr, &make_felt(10)), Some(make_felt(100)));
        assert_eq!(cache.get_storage(10, &addr, &make_felt(20)), Some(make_felt(200)));

        // Nonce should be cached
        assert_eq!(cache.get_nonce(10, &addr), Some(make_felt(5)));

        // Class hash should be cached
        assert_eq!(cache.get_class_hash(10, &addr), Some(make_felt(999)));

        // Class should be auto-registered
        assert!(cache.should_cache_class(&make_felt(999)));
    }

    #[test]
    fn test_invalidate_from_block() {
        let addr = make_felt(1);
        let config = make_config_with_contracts(vec![addr]);
        let cache = ContractCache::new(config);

        // Add entries at different blocks
        cache.put_storage(5, &addr, &make_felt(1), make_felt(100));
        cache.put_storage(10, &addr, &make_felt(2), make_felt(200));
        cache.put_storage(15, &addr, &make_felt(3), make_felt(300));
        cache.put_nonce(5, &addr, make_felt(1));
        cache.put_nonce(12, &addr, make_felt(2)); // This will overwrite

        // Invalidate from block 10
        cache.invalidate_from_block(10);

        // Entry at block 5 should still be there
        assert_eq!(cache.get_storage(5, &addr, &make_felt(1)), Some(make_felt(100)));

        // Entries at blocks 10 and 15 should be gone
        assert!(cache.get_storage(15, &addr, &make_felt(2)).is_none());
        assert!(cache.get_storage(15, &addr, &make_felt(3)).is_none());
    }

    #[test]
    fn test_memory_tracking() {
        let addr = make_felt(1);
        let config = make_config_with_contracts(vec![addr]);
        let cache = ContractCache::new(config);

        assert_eq!(cache.memory_bytes(), 0);

        cache.put_storage(0, &addr, &make_felt(1), make_felt(100));
        assert_eq!(cache.memory_bytes(), STORAGE_ENTRY_SIZE);

        cache.put_nonce(0, &addr, make_felt(1));
        assert_eq!(cache.memory_bytes(), STORAGE_ENTRY_SIZE + NONCE_ENTRY_SIZE);
    }

    #[test]
    fn test_lru_eviction() {
        let addr = make_felt(1);
        // Very small memory limit to trigger eviction
        let config = ContractCacheConfig {
            enabled: true,
            cached_contract_addresses: [addr].into_iter().collect(),
            max_memory_bytes: STORAGE_ENTRY_SIZE * 3, // Only allow ~3 entries
        };
        let cache = ContractCache::new(config);

        // Add more entries than the limit allows
        for i in 0..10 {
            cache.put_storage(0, &addr, &make_felt(i), make_felt(i * 100));
        }

        // Some entries should have been evicted
        assert!(cache.evictions() > 0);
        assert!(cache.storage_entry_count() < 10);
    }

    #[test]
    fn test_entry_counts() {
        let addr = make_felt(1);
        let config = make_config_with_contracts(vec![addr]);
        let cache = ContractCache::new(config);

        assert_eq!(cache.storage_entry_count(), 0);
        assert_eq!(cache.nonce_entry_count(), 0);
        assert_eq!(cache.class_hash_entry_count(), 0);

        cache.put_storage(0, &addr, &make_felt(1), make_felt(100));
        cache.put_storage(0, &addr, &make_felt(2), make_felt(200));
        cache.put_nonce(0, &addr, make_felt(1));
        cache.put_class_hash(0, &addr, make_felt(999));

        assert_eq!(cache.storage_entry_count(), 2);
        assert_eq!(cache.nonce_entry_count(), 1);
        assert_eq!(cache.class_hash_entry_count(), 1);
    }
}
