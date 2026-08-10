use std::cell::RefCell;
use std::collections::HashSet;
use std::env;
use std::sync::OnceLock;

use starknet_types_core::felt::Felt;

#[derive(Default)]
struct HashAggStats {
    pedersen_calls: u64,
    pedersen_hits: u64,
    pedersen_misses: u64,
    pedersen_inputs: HashSet<(Felt, Felt)>,
    poseidon_calls: u64,
    poseidon_inputs: HashSet<Vec<Felt>>,
    sn_keccak_calls: u64,
    sn_keccak_hits: u64,
    sn_keccak_misses: u64,
    sn_keccak_inputs: HashSet<Vec<u8>>,
    key_cache_hits: u64,
    key_cache_misses: u64,
    ctx_reads_total: u64,
    ctx_read_cache_hits: u64,
    ctx_write_hits: u64,
    ctx_backend_reads: u64,
    ctx_writes_total: u64,
}

#[derive(Default, Clone, Copy)]
pub struct HashAggSnapshot {
    pub pedersen_calls: u64,
    pub pedersen_hits: u64,
    pub pedersen_misses: u64,
    pub pedersen_inputs: u64,
    pub poseidon_calls: u64,
    pub poseidon_inputs: u64,
    pub sn_keccak_calls: u64,
    pub sn_keccak_hits: u64,
    pub sn_keccak_misses: u64,
    pub sn_keccak_inputs: u64,
    pub key_cache_hits: u64,
    pub key_cache_misses: u64,
    pub ctx_reads_total: u64,
    pub ctx_read_cache_hits: u64,
    pub ctx_write_hits: u64,
    pub ctx_backend_reads: u64,
    pub ctx_writes_total: u64,
}

#[derive(Clone, Copy)]
pub enum CtxReadSource {
    WriteCache,
    ReadCache,
    Backend,
}

thread_local! {
    static STATS: RefCell<HashAggStats> = RefCell::new(HashAggStats::default());
}

static HASH_AGG_ENABLED: OnceLock<bool> = OnceLock::new();

#[inline]
pub fn enabled() -> bool {
    *HASH_AGG_ENABLED
        .get_or_init(|| env::var("HASH_AGG_LOGS").map(|v| v == "1" || v.eq_ignore_ascii_case("true")).unwrap_or(false))
}

#[inline]
pub fn reset() {
    if !enabled() {
        return;
    }
    STATS.with(|stats| *stats.borrow_mut() = HashAggStats::default());
}

#[inline]
pub fn snapshot() -> HashAggSnapshot {
    if !enabled() {
        return HashAggSnapshot::default();
    }
    STATS.with(|stats| {
        let stats = stats.borrow();
        HashAggSnapshot {
            pedersen_calls: stats.pedersen_calls,
            pedersen_hits: stats.pedersen_hits,
            pedersen_misses: stats.pedersen_misses,
            pedersen_inputs: stats.pedersen_inputs.len() as u64,
            poseidon_calls: stats.poseidon_calls,
            poseidon_inputs: stats.poseidon_inputs.len() as u64,
            sn_keccak_calls: stats.sn_keccak_calls,
            sn_keccak_hits: stats.sn_keccak_hits,
            sn_keccak_misses: stats.sn_keccak_misses,
            sn_keccak_inputs: stats.sn_keccak_inputs.len() as u64,
            key_cache_hits: stats.key_cache_hits,
            key_cache_misses: stats.key_cache_misses,
            ctx_reads_total: stats.ctx_reads_total,
            ctx_read_cache_hits: stats.ctx_read_cache_hits,
            ctx_write_hits: stats.ctx_write_hits,
            ctx_backend_reads: stats.ctx_backend_reads,
            ctx_writes_total: stats.ctx_writes_total,
        }
    })
}

#[inline]
pub fn record_pedersen_call(left: Felt, right: Felt) {
    if !enabled() {
        return;
    }
    STATS.with(|stats| {
        let mut stats = stats.borrow_mut();
        stats.pedersen_calls = stats.pedersen_calls.saturating_add(1);
        stats.pedersen_inputs.insert((left, right));
    });
}

#[inline]
pub fn record_pedersen_cache_hit() {
    if !enabled() {
        return;
    }
    STATS.with(|stats| {
        let mut stats = stats.borrow_mut();
        stats.pedersen_hits = stats.pedersen_hits.saturating_add(1);
    });
}

#[inline]
pub fn record_pedersen_cache_miss() {
    if !enabled() {
        return;
    }
    STATS.with(|stats| {
        let mut stats = stats.borrow_mut();
        stats.pedersen_misses = stats.pedersen_misses.saturating_add(1);
    });
}

#[inline]
pub fn record_poseidon_call(values: &[Felt]) {
    if !enabled() {
        return;
    }
    STATS.with(|stats| {
        let mut stats = stats.borrow_mut();
        stats.poseidon_calls = stats.poseidon_calls.saturating_add(1);
        stats.poseidon_inputs.insert(values.to_vec());
    });
}

#[inline]
pub fn record_sn_keccak_call(data: &[u8], hit: bool) {
    if !enabled() {
        return;
    }
    STATS.with(|stats| {
        let mut stats = stats.borrow_mut();
        stats.sn_keccak_calls = stats.sn_keccak_calls.saturating_add(1);
        if hit {
            stats.sn_keccak_hits = stats.sn_keccak_hits.saturating_add(1);
        } else {
            stats.sn_keccak_misses = stats.sn_keccak_misses.saturating_add(1);
        }
        stats.sn_keccak_inputs.insert(data.to_vec());
    });
}

#[inline]
pub fn record_key_cache_hit() {
    if !enabled() {
        return;
    }
    STATS.with(|stats| {
        let mut stats = stats.borrow_mut();
        stats.key_cache_hits = stats.key_cache_hits.saturating_add(1);
    });
}

#[inline]
pub fn record_key_cache_miss() {
    if !enabled() {
        return;
    }
    STATS.with(|stats| {
        let mut stats = stats.borrow_mut();
        stats.key_cache_misses = stats.key_cache_misses.saturating_add(1);
    });
}

#[inline]
pub fn record_ctx_read(source: CtxReadSource) {
    if !enabled() {
        return;
    }
    STATS.with(|stats| {
        let mut stats = stats.borrow_mut();
        stats.ctx_reads_total = stats.ctx_reads_total.saturating_add(1);
        match source {
            CtxReadSource::WriteCache => {
                stats.ctx_write_hits = stats.ctx_write_hits.saturating_add(1);
            }
            CtxReadSource::ReadCache => {
                stats.ctx_read_cache_hits = stats.ctx_read_cache_hits.saturating_add(1);
            }
            CtxReadSource::Backend => {
                stats.ctx_backend_reads = stats.ctx_backend_reads.saturating_add(1);
            }
        }
    });
}

#[inline]
pub fn record_ctx_write() {
    if !enabled() {
        return;
    }
    STATS.with(|stats| {
        let mut stats = stats.borrow_mut();
        stats.ctx_writes_total = stats.ctx_writes_total.saturating_add(1);
    });
}
