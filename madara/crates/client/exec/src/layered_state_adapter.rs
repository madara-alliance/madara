use crate::execution_read_cache::ExecutionReadCache;
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
use mc_db::{rocksdb::RocksDBStorage, MadaraBackend, MadaraStorageRead};
use mp_block::header::GasPrices;
use mp_convert::Felt;
use starknet_api::{
    contract_class::ContractClass as ApiContractClass,
    core::{ClassHash, CompiledClassHash, ContractAddress, Nonce},
    state::StorageKey,
};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::Arc,
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

        let gas_prices = if let Some(custom_header) = backend.get_custom_header().filter(|h| h.block_n == block_number) {
            custom_header.gas_prices
        } else {
            let l1_gas_quote = backend
                .get_last_l1_gas_quote()
                .context("No L1 gas quote available. Ensure that the L1 gas quote is set before calculating gas prices.")?;

            if let Some(block) = view.block_view_on_latest_confirmed() {
                let block_info = block.get_block_info()?;
                let previous_strk_l2_gas_price = block_info.header.gas_prices.strk_l2_gas_price;
                let previous_l2_gas_used = block_info.total_l2_gas_used;

                backend.calculate_gas_prices(&l1_gas_quote, previous_strk_l2_gas_price, previous_l2_gas_used)?
            } else {
                backend.calculate_gas_prices(&l1_gas_quote, 0, 0)?
            }
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

    fn get_contract_scoped_with_cache<T, FLayered, FReadCache, FDb, FBackfill>(
        &self,
        contract_address: ContractAddress,
        cache_kind: &'static str,
        layered_lookup: FLayered,
        read_cache_lookup: FReadCache,
        db_lookup: FDb,
        read_cache_backfill: FBackfill,
    ) -> StateResult<T>
    where
        T: Copy,
        FLayered: Fn(&CacheByBlock) -> Option<T>,
        FReadCache: Fn(&ExecutionReadCache) -> Option<T>,
        FDb: Fn(&BlockifierStateAdapter<D>) -> StateResult<T>,
        FBackfill: Fn(&ExecutionReadCache, T),
    {
        if let Some(value) = self.cached_states_by_block_n.iter().find_map(layered_lookup) {
            return Ok(value);
        }

        if let Some(read_cache) = &self.read_cache {
            if read_cache.is_contract_enabled(contract_address) {
                if let Some(value) = read_cache_lookup(read_cache) {
                    exec_metrics().record_read_cache_hit(cache_kind);
                    return Ok(value);
                }
                exec_metrics().record_read_cache_miss(cache_kind);
            }
        }

        let value = db_lookup(&self.inner)?;
        if let Some(read_cache) = &self.read_cache {
            read_cache_backfill(read_cache, value);
        }
        Ok(value)
    }
}

impl<D: MadaraStorageRead> StateReader for LayeredStateAdapter<D> {
    fn get_storage_at(&self, contract_address: ContractAddress, key: StorageKey) -> StateResult<Felt> {
        self.get_contract_scoped_with_cache(
            contract_address,
            read_cache_kind::STORAGE,
            |s| s.state_diff.storage.get(&(contract_address, key)).copied(),
            |read_cache| read_cache.get_storage(contract_address, key),
            |inner| inner.get_storage_at(contract_address, key),
            |read_cache, value| read_cache.insert_storage_value(contract_address, key, value),
        )
    }
    fn get_nonce_at(&self, contract_address: ContractAddress) -> StateResult<Nonce> {
        self.get_contract_scoped_with_cache(
            contract_address,
            read_cache_kind::NONCE,
            |s| s.state_diff.nonces.get(&contract_address).copied(),
            |read_cache| read_cache.get_nonce(contract_address),
            |inner| inner.get_nonce_at(contract_address),
            |read_cache, value| read_cache.insert_nonce_value(contract_address, value),
        )
    }
    fn get_class_hash_at(&self, contract_address: ContractAddress) -> StateResult<ClassHash> {
        self.get_contract_scoped_with_cache(
            contract_address,
            read_cache_kind::CLASS_HASH,
            |s| s.state_diff.class_hashes.get(&contract_address).copied(),
            |read_cache| read_cache.get_class_hash(contract_address),
            |inner| inner.get_class_hash_at(contract_address),
            |read_cache, value| read_cache.insert_class_hash_value(contract_address, value),
        )
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
    use crate::metrics::test_counters;
    use blockifier::state::{cached_state::StateMaps, state_api::StateReader};
    use mc_db::{ExecutionReadCacheConfig, MadaraBackend, MadaraBackendConfig};
    use mp_block::{
        header::{BlockTimestamp, GasPrices, PreconfirmedHeader},
        FullBlockWithoutCommitments,
    };
    use mp_chain_config::{ChainConfig, L1DataAvailabilityMode, StarknetVersion};
    use mp_convert::{Felt, ToFelt};
    use mp_state_update::{ContractStorageDiffItem, StateDiff, StorageEntry};
    use std::sync::{atomic::Ordering, Arc};

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
                contracts: None,
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
            assert_eq!(cache.test_storage_len(), 0);
        }

        insert_confirmed_block_with_storage(&backend, 0, Felt::ONE, Felt::ONE, Felt::THREE);

        adaptor.finish_block(StateMaps::default(), Default::default(), Default::default()).unwrap();

        {
            let cache = adaptor.read_cache.as_ref().unwrap();
            assert_eq!(cache.get_storage(contract_address, storage_key), Some(Felt::THREE));
        }
    }

    #[tokio::test]
    async fn test_layered_state_adapter_read_cache_contract_filter() {
        let allowed_address = Felt::ONE.try_into().unwrap();
        let config = MadaraBackendConfig {
            execution_read_cache: ExecutionReadCacheConfig {
                enabled: true,
                contracts: Some(vec![allowed_address]),
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
        assert_eq!(cache.get_storage(allowed_address, storage_key), Some(Felt::THREE));
        assert_eq!(cache.get_storage(blocked_address, storage_key), None);
    }

    #[tokio::test]
    async fn test_execution_read_cache_metrics_harness_records_hit_miss_and_size() {
        let _metrics_guard = test_counters::acquire_and_reset();

        let config = MadaraBackendConfig {
            execution_read_cache: ExecutionReadCacheConfig {
                enabled: true,
                contracts: None,
                max_memory_bytes: 1024 * 1024,
            },
            ..Default::default()
        };

        let backend = MadaraBackend::open_for_testing_with_config(ChainConfig::madara_test().into(), config);
        backend.set_l1_gas_quote_for_testing();
        let adaptor = LayeredStateAdapter::new(backend).unwrap();

        let contract_address = Felt::ONE.try_into().unwrap();
        let storage_key = Felt::ONE.try_into().unwrap();

        // First call misses read-cache and backfills from DB; second call hits read-cache.
        let _ = adaptor.get_storage_at(contract_address, storage_key).unwrap();
        let _ = adaptor.get_storage_at(contract_address, storage_key).unwrap();

        assert!(test_counters::READ_CACHE_MISSES_TOTAL.load(Ordering::Relaxed) >= 1);
        assert!(test_counters::READ_CACHE_HITS_TOTAL.load(Ordering::Relaxed) >= 1);
        assert!(test_counters::READ_CACHE_SIZE_RECORDS.load(Ordering::Relaxed) >= 1);
        assert!(test_counters::READ_CACHE_SIZE_LAST.load(Ordering::Relaxed) > 0);
    }
}
