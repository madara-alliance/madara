use std::cell::RefCell;
use std::collections::HashSet;

use crate::core::types::{ContractAddress, StorageKey};

#[derive(Default)]
struct StorageAggStats {
    ctx_reads_total: u64,
    ctx_read_cache_hits: u64,
    ctx_write_cache_hits: u64,
    ctx_backend_reads: u64,
    ctx_writes_total: u64,
    ctx_read_cache_us: u64,
    ctx_write_cache_us: u64,
    ctx_backend_us: u64,
    ctx_write_us: u64,
    ctx_read_cache_keys: HashSet<(ContractAddress, StorageKey)>,
    ctx_write_cache_keys: HashSet<(ContractAddress, StorageKey)>,
    ctx_backend_keys: HashSet<(ContractAddress, StorageKey)>,
    ctx_write_keys: HashSet<(ContractAddress, StorageKey)>,
}

#[derive(Default, Clone, Copy)]
pub struct StorageAggSnapshot {
    pub ctx_reads_total: u64,
    pub ctx_read_cache_hits: u64,
    pub ctx_write_cache_hits: u64,
    pub ctx_backend_reads: u64,
    pub ctx_writes_total: u64,
    pub ctx_read_cache_unique: u64,
    pub ctx_write_cache_unique: u64,
    pub ctx_backend_unique: u64,
    pub ctx_write_unique: u64,
    pub ctx_read_cache_us: u64,
    pub ctx_write_cache_us: u64,
    pub ctx_backend_us: u64,
    pub ctx_write_us: u64,
}

#[derive(Clone, Copy)]
pub enum CtxReadLayer {
    WriteCache,
    ReadCache,
    Backend,
}

thread_local! {
    static STATS: RefCell<StorageAggStats> = RefCell::new(StorageAggStats::default());
}

#[inline]
pub fn enabled() -> bool {
    crate::config::storage_agg_logs_enabled()
}

#[inline]
pub fn reset() {
    if !enabled() {
        return;
    }
    STATS.with(|stats| *stats.borrow_mut() = StorageAggStats::default());
}

#[inline]
pub fn snapshot() -> StorageAggSnapshot {
    if !enabled() {
        return StorageAggSnapshot::default();
    }
    STATS.with(|stats| {
        let stats = stats.borrow();
        StorageAggSnapshot {
            ctx_reads_total: stats.ctx_reads_total,
            ctx_read_cache_hits: stats.ctx_read_cache_hits,
            ctx_write_cache_hits: stats.ctx_write_cache_hits,
            ctx_backend_reads: stats.ctx_backend_reads,
            ctx_writes_total: stats.ctx_writes_total,
            ctx_read_cache_unique: stats.ctx_read_cache_keys.len() as u64,
            ctx_write_cache_unique: stats.ctx_write_cache_keys.len() as u64,
            ctx_backend_unique: stats.ctx_backend_keys.len() as u64,
            ctx_write_unique: stats.ctx_write_keys.len() as u64,
            ctx_read_cache_us: stats.ctx_read_cache_us,
            ctx_write_cache_us: stats.ctx_write_cache_us,
            ctx_backend_us: stats.ctx_backend_us,
            ctx_write_us: stats.ctx_write_us,
        }
    })
}

#[inline]
pub fn record_ctx_read(layer: CtxReadLayer, contract: ContractAddress, key: StorageKey, elapsed_us: u64) {
    if !enabled() {
        return;
    }
    STATS.with(|stats| {
        let mut stats = stats.borrow_mut();
        stats.ctx_reads_total = stats.ctx_reads_total.saturating_add(1);
        match layer {
            CtxReadLayer::WriteCache => {
                stats.ctx_write_cache_hits = stats.ctx_write_cache_hits.saturating_add(1);
                stats.ctx_write_cache_us = stats.ctx_write_cache_us.saturating_add(elapsed_us);
                stats.ctx_write_cache_keys.insert((contract, key));
            }
            CtxReadLayer::ReadCache => {
                stats.ctx_read_cache_hits = stats.ctx_read_cache_hits.saturating_add(1);
                stats.ctx_read_cache_us = stats.ctx_read_cache_us.saturating_add(elapsed_us);
                stats.ctx_read_cache_keys.insert((contract, key));
            }
            CtxReadLayer::Backend => {
                stats.ctx_backend_reads = stats.ctx_backend_reads.saturating_add(1);
                stats.ctx_backend_us = stats.ctx_backend_us.saturating_add(elapsed_us);
                stats.ctx_backend_keys.insert((contract, key));
            }
        }
    });
}

#[inline]
pub fn record_ctx_write(contract: ContractAddress, key: StorageKey, elapsed_us: u64) {
    if !enabled() {
        return;
    }
    STATS.with(|stats| {
        let mut stats = stats.borrow_mut();
        stats.ctx_writes_total = stats.ctx_writes_total.saturating_add(1);
        stats.ctx_write_us = stats.ctx_write_us.saturating_add(elapsed_us);
        stats.ctx_write_keys.insert((contract, key));
    });
}
