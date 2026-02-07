use anyhow::{anyhow, bail, Context, Result};
use clap::Parser;
use mc_db::read_hook::{clear_read_hook, set_context_block, set_read_hook, ReadEvent, ReadHook, ReadOp};
use mc_db::rocksdb::trie::{bonsai_db_ops_set_enabled, bonsai_db_ops_snapshot, BonsaiDbOpsSnapshot, BONSAI_CF_NAMES};
use mc_db::rocksdb::{RocksDBConfig, RocksDBStorage};
use mc_db::storage::StorageChainTip;
use mc_db::{MadaraStorageRead, MadaraStorageWrite};
use mp_convert::Felt;
use mp_state_update::{
    ContractStorageDiffItem, DeclaredClassItem, DeployedContractItem, MigratedClassItem, NonceUpdate,
    ReplacedClassItem, StateDiff, StorageEntry,
};
use rayon::ThreadPool;
use reqwest::blocking::Client as BlockingClient;
use serde::de::DeserializeOwned;
use serde::Serialize;
use starknet_core::types::{
    ContractStorageDiffItem as RpcContractStorageDiffItem, DeclaredClassItem as RpcDeclaredClassItem,
    DeployedContractItem as RpcDeployedContractItem, MaybePreConfirmedBlockWithTxHashes, MaybePreConfirmedStateUpdate,
    NonceUpdate as RpcNonceUpdate, ReplacedClassItem as RpcReplacedClassItem, StateDiff as RpcStateDiff,
    StateUpdate as RpcStateUpdate, StorageEntry as RpcStorageEntry,
};
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;
use std::time::Instant;
use std::{collections::HashMap, io::BufRead};
use tracing::{info, warn};

#[derive(Debug, Parser)]
#[command(name = "parallel-merklelization", about = "Baseline runner for parallel merklelization POC")]
struct Args {
    #[arg(long, env = "PARALLEL_MERKLELIZATION_DB_PATH", value_name = "PATH")]
    db_path: PathBuf,
    #[arg(long, env = "PARALLEL_MERKLELIZATION_SOURCE_DB_PATH", value_name = "PATH")]
    source_db_path: Option<PathBuf>,
    /// RPC URL for root validation and/or RPC-based state diffs / read fallback.
    /// Required unless running `--synthetic`.
    #[arg(long, env = "PARALLEL_MERKLELIZATION_RPC_URL", value_name = "URL")]
    rpc_url: Option<String>,
    #[arg(long, value_name = "BLOCK")]
    start_block: Option<u64>,
    #[arg(long, value_name = "BLOCK", conflicts_with = "block_count")]
    end_block: Option<u64>,
    #[arg(long, value_name = "N", conflicts_with = "end_block")]
    block_count: Option<u64>,
    #[arg(long, value_name = "DIR", default_value = "./artifacts/parallel-merklelization/phase1")]
    out_dir: PathBuf,
    #[arg(long, value_name = "PATH")]
    read_map: Option<PathBuf>,
    /// Write a deduplicated read map after the run.
    #[arg(long, value_name = "PATH")]
    read_map_out: Option<PathBuf>,
    #[arg(long)]
    require_read_map: bool,
    /// Fetch missing read-map entries via RPC and cache them in-memory.
    #[arg(long)]
    rpc_read_fallback: bool,
    /// Source for state diffs (db or rpc).
    #[arg(long, value_enum, default_value_t = StateDiffSource::Db)]
    state_diff_source: StateDiffSource,
    /// Open source DB in read-only mode (allows shared source DB across processes).
    #[arg(long)]
    source_read_only: bool,
    /// Apply a squashed state diff for the range instead of per-block diffs.
    #[arg(long)]
    squash_range: bool,
    /// Pre-range block used for compression (defaults to start_block - 1).
    #[arg(long, value_name = "BLOCK")]
    pre_range_block: Option<u64>,
    /// Apply state diffs to non-bonsai columns on the target DB before merklization.
    #[arg(long)]
    apply_state_diff_columns: bool,
    /// Skip end-block root validation against RPC. Useful for benchmarks where validation is run separately.
    #[arg(long)]
    skip_rpc_root_check: bool,
    /// Record per-apply Bonsai DB operation counters (get/insert/remove/etc) from the Bonsai RocksDB wrapper.
    #[arg(long)]
    bonsai_db_ops_metrics: bool,
    #[arg(long)]
    force: bool,

    /// Run a synthetic merkleization micro-benchmark (no RPC, no state diffs from DB).
    /// Useful to validate timing equations with controlled edge cases.
    #[arg(long)]
    synthetic: bool,

    /// Number of contracts with storage diffs.
    #[arg(long, value_name = "N", default_value_t = 1, requires = "synthetic")]
    synthetic_contracts: u64,
    /// Storage keys updated per contract.
    #[arg(long, value_name = "N", default_value_t = 30, requires = "synthetic")]
    synthetic_storage_keys_per_contract: u64,
    /// Number of deployed contracts included in the state diff.
    #[arg(long, value_name = "N", default_value_t = 0, requires = "synthetic")]
    synthetic_deployed_contracts: u64,
    /// Extra contracts to touch via nonce updates only (no storage diffs).
    /// This is useful to keep `touched_contracts` in the same range as mainnet diffs while holding storage keys constant.
    #[arg(long, value_name = "N", default_value_t = 0, requires = "synthetic")]
    synthetic_nonce_only_contracts: u64,
    /// Warmup iterations (not recorded) to stabilize caches for warm-worker timings.
    #[arg(long, value_name = "N", default_value_t = 50, requires = "synthetic")]
    synthetic_warmup_iters: u64,
    /// Recorded iterations.
    #[arg(long, value_name = "N", default_value_t = 200, requires = "synthetic")]
    synthetic_iters: u64,
    /// RNG seed for deterministic synthetic workloads.
    #[arg(long, value_name = "SEED", default_value_t = 1, requires = "synthetic")]
    synthetic_seed: u64,
    /// Storage key generation pattern.
    #[arg(long, value_enum, default_value_t = SyntheticKeyPattern::Random, requires = "synthetic")]
    synthetic_key_pattern: SyntheticKeyPattern,
    /// Reuse the same contract addresses + storage keys across iterations (only values change).
    #[arg(long, requires = "synthetic")]
    synthetic_fixed_shape: bool,
    /// Reuse an existing RocksDB at `--db-path` instead of creating a fresh empty DB.
    /// This is required if you want synthetic timings comparable to a large production trie.
    #[arg(long, requires = "synthetic")]
    synthetic_reuse_db: bool,
    /// First block number to write as `latest_applied_trie_update` for synthetic applies.
    /// Defaults to `db.latest_applied_trie_update + 1` when reusing a DB, otherwise `1`.
    #[arg(long, value_name = "BLOCK", requires = "synthetic")]
    synthetic_start_block: Option<u64>,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum, PartialEq, Eq)]
enum StateDiffSource {
    Db,
    Rpc,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum, PartialEq, Eq)]
enum SyntheticKeyPattern {
    /// Fully random 251-bit Felts (per key slot) for the synthetic contract storage keys.
    Random,
    /// Keys share a common prefix (high bytes) but differ in the suffix; this tends to increase shared trie ancestry.
    CommonPrefix,
    /// Sequential keys: 1,2,3,... (masked to 251 bits). Good for deterministic debugging.
    Sequential,
}

#[derive(Default)]
struct ReadCounters {
    total: AtomicU64,
    storage_at: AtomicU64,
    nonce_at: AtomicU64,
    class_hash_at: AtomicU64,
    override_hits: AtomicU64,
    override_misses: AtomicU64,
}

#[derive(Default)]
struct RpcStats {
    read_fallback_count: AtomicU64,
    read_fallback_total_ms: AtomicU64,
    state_diff_count: AtomicU64,
    state_diff_total_ms: AtomicU64,
}

struct ReadMap {
    entries: RwLock<HashMap<String, Option<Felt>>>,
}

impl ReadMap {
    fn load(path: &Path) -> Result<Self> {
        let file = File::open(path).with_context(|| format!("Opening read map at {}", path.display()))?;
        let reader = std::io::BufReader::new(file);
        let mut entries: HashMap<String, Option<Felt>> = HashMap::new();
        let mut dup_mismatches = 0u64;

        for (idx, line) in reader.lines().enumerate() {
            let line = line.with_context(|| format!("Reading read map line {}", idx + 1))?;
            if line.trim().is_empty() {
                continue;
            }
            let event: ReadEvent = serde_json::from_str(&line)
                .with_context(|| format!("Parsing read map line {} as ReadEvent", idx + 1))?;
            let key = serde_json::to_string(&event.op).context("Serializing ReadOp key")?;
            if let Some(existing) = entries.get(&key) {
                if existing != &event.value {
                    dup_mismatches += 1;
                }
                continue;
            }
            entries.insert(key, event.value);
        }

        if dup_mismatches > 0 {
            warn!("Read map contains {} duplicate keys with mismatched values", dup_mismatches);
        }

        Ok(Self { entries: RwLock::new(entries) })
    }

    fn lookup(&self, op: &ReadOp) -> Option<Option<Felt>> {
        let key = match serde_json::to_string(op) {
            Ok(key) => key,
            Err(err) => {
                warn!("Failed to serialize ReadOp for read map lookup: {err}");
                return None;
            }
        };
        let guard = self.entries.read().expect("read map lock poisoned");
        guard.get(&key).cloned()
    }

    fn insert(&self, op: &ReadOp, value: Option<Felt>) {
        let key = match serde_json::to_string(op) {
            Ok(key) => key,
            Err(err) => {
                warn!("Failed to serialize ReadOp for read map insert: {err}");
                return;
            }
        };
        let mut guard = self.entries.write().expect("read map lock poisoned");
        guard.entry(key).or_insert(value);
    }

    fn from_entries(entries: HashMap<String, Option<Felt>>) -> Self {
        Self { entries: RwLock::new(entries) }
    }
}

struct ReadMapWriter {
    entries: Mutex<HashMap<String, Option<Felt>>>,
    mismatches: AtomicU64,
}

impl ReadMapWriter {
    fn new() -> Self {
        Self { entries: Mutex::new(HashMap::new()), mismatches: AtomicU64::new(0) }
    }

    fn record(&self, event: &ReadEvent) {
        let key = match serde_json::to_string(&event.op) {
            Ok(key) => key,
            Err(_) => return,
        };
        let mut guard = self.entries.lock().expect("read map writer lock poisoned");
        if let Some(existing) = guard.get(&key) {
            if existing != &event.value {
                self.mismatches.fetch_add(1, Ordering::Relaxed);
            }
            return;
        }
        guard.insert(key, event.value);
    }

    fn write_to(&self, path: &Path) -> Result<()> {
        let guard = self.entries.lock().expect("read map writer lock poisoned");
        let mut writer = BufWriter::new(File::create(path)?);
        for (key, value) in guard.iter() {
            let op: ReadOp = serde_json::from_str(key).context("Deserializing read map key")?;
            let event =
                ReadEvent { op, value: *value, source: mc_db::read_hook::ReadSource::Override, context_block: None };
            serde_json::to_writer(&mut writer, &event)?;
            writer.write_all(b"\n")?;
        }
        writer.flush()?;
        Ok(())
    }

    fn mismatches(&self) -> u64 {
        self.mismatches.load(Ordering::Relaxed)
    }
}

struct RpcReadFallback {
    client: BlockingClient,
    url: String,
    stats: Arc<RpcStats>,
}

impl RpcReadFallback {
    fn new(url: &str, stats: Arc<RpcStats>) -> Result<Self> {
        Ok(Self { client: BlockingClient::new(), url: url.to_string(), stats })
    }

    fn read(&self, op: &ReadOp) -> Result<Option<Felt>> {
        let start = Instant::now();
        let result = match op {
            ReadOp::GetStorageAt { block_n, contract_address, key } => {
                let params = serde_json::json!([format_felt(*contract_address), format_felt(*key), block_id(*block_n)]);
                rpc_call_felt(&self.client, &self.url, "starknet_getStorageAt", params)
            }
            ReadOp::GetContractNonceAt { block_n, contract_address } => {
                let params = serde_json::json!([block_id(*block_n), format_felt(*contract_address)]);
                rpc_call_felt(&self.client, &self.url, "starknet_getNonce", params)
            }
            ReadOp::GetContractClassHashAt { block_n, contract_address } => {
                let params = serde_json::json!([block_id(*block_n), format_felt(*contract_address)]);
                rpc_call_felt(&self.client, &self.url, "starknet_getClassHashAt", params)
            }
        };
        let elapsed = start.elapsed();
        self.stats.read_fallback_count.fetch_add(1, Ordering::Relaxed);
        self.stats.read_fallback_total_ms.fetch_add(elapsed.as_millis() as u64, Ordering::Relaxed);
        result
    }
}

struct JsonlReadRecorder {
    writer: Mutex<BufWriter<File>>,
    counters: ReadCounters,
    read_map: Option<ReadMap>,
    read_map_writer: Option<Arc<ReadMapWriter>>,
    rpc_fallback: Option<RpcReadFallback>,
}

impl JsonlReadRecorder {
    fn new(
        path: &Path,
        read_map: Option<ReadMap>,
        read_map_writer: Option<Arc<ReadMapWriter>>,
        rpc_fallback: Option<RpcReadFallback>,
    ) -> Result<Self> {
        let file = File::create(path).with_context(|| format!("Creating read log at {}", path.display()))?;
        Ok(Self {
            writer: Mutex::new(BufWriter::new(file)),
            counters: ReadCounters::default(),
            read_map,
            read_map_writer,
            rpc_fallback,
        })
    }

    fn flush(&self) -> Result<()> {
        let mut guard = self.writer.lock().expect("read log lock poisoned");
        guard.flush().context("Flushing read log")
    }

    fn stats(&self) -> ReadStats {
        ReadStats {
            total: self.counters.total.load(Ordering::Relaxed),
            storage_at: self.counters.storage_at.load(Ordering::Relaxed),
            nonce_at: self.counters.nonce_at.load(Ordering::Relaxed),
            class_hash_at: self.counters.class_hash_at.load(Ordering::Relaxed),
            override_hits: self.counters.override_hits.load(Ordering::Relaxed),
            override_misses: self.counters.override_misses.load(Ordering::Relaxed),
        }
    }
}

impl ReadHook for JsonlReadRecorder {
    fn override_read(&self, op: &ReadOp) -> Option<Option<Felt>> {
        let read_map = match self.read_map.as_ref() {
            Some(map) => map,
            None => return None,
        };
        if let Some(value) = read_map.lookup(op) {
            self.counters.override_hits.fetch_add(1, Ordering::Relaxed);
            return Some(value);
        }

        self.counters.override_misses.fetch_add(1, Ordering::Relaxed);

        let fallback = self.rpc_fallback.as_ref()?;
        match fallback.read(op) {
            Ok(value) => {
                read_map.insert(op, value);
                self.counters.override_hits.fetch_add(1, Ordering::Relaxed);
                Some(value)
            }
            Err(err) => panic!("RPC read fallback failed for {:?}: {err}", op),
        }
    }

    fn on_read(&self, event: ReadEvent) {
        self.counters.total.fetch_add(1, Ordering::Relaxed);
        match event.op {
            ReadOp::GetStorageAt { .. } => {
                self.counters.storage_at.fetch_add(1, Ordering::Relaxed);
            }
            ReadOp::GetContractNonceAt { .. } => {
                self.counters.nonce_at.fetch_add(1, Ordering::Relaxed);
            }
            ReadOp::GetContractClassHashAt { .. } => {
                self.counters.class_hash_at.fetch_add(1, Ordering::Relaxed);
            }
        }

        let mut guard = self.writer.lock().expect("read log lock poisoned");
        if let Err(err) = serde_json::to_writer(&mut *guard, &event) {
            panic!("Failed to write read event JSON: {err}");
        }
        if let Err(err) = guard.write_all(b"\n") {
            panic!("Failed to write read event newline: {err}");
        }

        if let Some(writer) = &self.read_map_writer {
            writer.record(&event);
        }
    }
}

#[derive(Serialize)]
struct RootRecord {
    block_number: u64,
    state_root: Felt,
}

#[derive(Serialize)]
struct RunSummary {
    db_path: String,
    source_db_path: String,
    source_read_only: bool,
    state_diff_source: String,
    rpc_url: String,
    start_block: u64,
    end_block: u64,
    total_blocks: u64,
    end_block_state_root: Felt,
    rpc_end_block_state_root: Option<Felt>,
    /// Total wall-clock time for the whole run (including setup and validation).
    wall_clock_total_ms: u64,
    /// Wall-clock time spent in the "compute" phase (squash + apply + local writes), excluding RPC root validation.
    wall_clock_compute_ms: u64,
    /// Wall-clock time spent validating end-root via RPC (0 if skipped).
    rpc_root_check_ms: u64,
    /// Sum of `MerklizationTimings.total` across all applies in this run.
    merklization_total_ms: u64,
    /// Time spent building a squashed diff (0 if not squashing).
    squash_total_ms: u64,
    read_stats: ReadStats,
    rpc_read_fallback_count: u64,
    rpc_read_fallback_total_ms: u64,
    rpc_state_diff_count: u64,
    rpc_state_diff_total_ms: u64,
    bonsai_db_ops_metrics_enabled: bool,
    bonsai_cf_names: Vec<String>,
    calls_path: String,
    roots_path: String,
    applies_path: String,
    read_map_path: Option<String>,
}

#[derive(Serialize)]
struct ReadStats {
    total: u64,
    storage_at: u64,
    nonce_at: u64,
    class_hash_at: u64,
    override_hits: u64,
    override_misses: u64,
}

#[derive(Debug, Clone, Copy, Default)]
struct RpcSnapshot {
    read_fallback_count: u64,
    read_fallback_total_ms: u64,
    state_diff_count: u64,
    state_diff_total_ms: u64,
}

impl RpcSnapshot {
    fn capture(stats: &RpcStats) -> Self {
        Self {
            read_fallback_count: stats.read_fallback_count.load(Ordering::Relaxed),
            read_fallback_total_ms: stats.read_fallback_total_ms.load(Ordering::Relaxed),
            state_diff_count: stats.state_diff_count.load(Ordering::Relaxed),
            state_diff_total_ms: stats.state_diff_total_ms.load(Ordering::Relaxed),
        }
    }
}

fn read_stats_delta(after: ReadStats, before: ReadStats) -> ReadStats {
    ReadStats {
        total: after.total.saturating_sub(before.total),
        storage_at: after.storage_at.saturating_sub(before.storage_at),
        nonce_at: after.nonce_at.saturating_sub(before.nonce_at),
        class_hash_at: after.class_hash_at.saturating_sub(before.class_hash_at),
        override_hits: after.override_hits.saturating_sub(before.override_hits),
        override_misses: after.override_misses.saturating_sub(before.override_misses),
    }
}

#[derive(Debug, Default)]
struct StateDiffStats {
    touched_contracts: u64,
    storage_diffs_contracts: u64,
    storage_entries: u64,
    unique_storage_keys: u64,
    max_storage_entries_per_contract: u64,
    nonce_updates: u64,
    deployed_contracts: u64,
    replaced_classes: u64,
    declared_classes: u64,
    migrated_compiled_classes: u64,
    deprecated_declared_classes: u64,
}

fn state_diff_stats(diff: &StateDiff) -> StateDiffStats {
    let mut touched: std::collections::HashSet<Felt> = std::collections::HashSet::new();
    let mut unique_storage: std::collections::HashSet<(Felt, Felt)> = std::collections::HashSet::new();

    let mut storage_entries: u64 = 0;
    let mut max_per_contract: u64 = 0;
    for contract_diff in &diff.storage_diffs {
        touched.insert(contract_diff.address);
        let per_contract = contract_diff.storage_entries.len() as u64;
        storage_entries += per_contract;
        max_per_contract = max_per_contract.max(per_contract);
        for entry in &contract_diff.storage_entries {
            unique_storage.insert((contract_diff.address, entry.key));
        }
    }

    for n in &diff.nonces {
        touched.insert(n.contract_address);
    }
    for d in &diff.deployed_contracts {
        touched.insert(d.address);
    }
    for r in &diff.replaced_classes {
        touched.insert(r.contract_address);
    }

    StateDiffStats {
        touched_contracts: touched.len() as u64,
        storage_diffs_contracts: diff.storage_diffs.len() as u64,
        storage_entries,
        unique_storage_keys: unique_storage.len() as u64,
        max_storage_entries_per_contract: max_per_contract,
        nonce_updates: diff.nonces.len() as u64,
        deployed_contracts: diff.deployed_contracts.len() as u64,
        replaced_classes: diff.replaced_classes.len() as u64,
        declared_classes: diff.declared_classes.len() as u64,
        migrated_compiled_classes: diff.migrated_compiled_classes.len() as u64,
        deprecated_declared_classes: diff.old_declared_contracts.len() as u64,
    }
}

#[derive(Serialize)]
struct ApplyRecord {
    apply_index: u64,
    apply_mode: String,
    start_block: u64,
    end_block: u64,
    block_count: u64,
    touched_contracts: u64,
    storage_diffs_contracts: u64,
    storage_entries: u64,
    unique_storage_keys: u64,
    max_storage_entries_per_contract: u64,
    nonce_updates: u64,
    deployed_contracts: u64,
    replaced_classes: u64,
    declared_classes: u64,
    migrated_compiled_classes: u64,
    deprecated_declared_classes: u64,
    apply_wall_clock_ms: u64,
    merklization_ms: u64,
    contract_trie_root_ms: u64,
    class_trie_root_ms: u64,
    contract_storage_commit_ms: u64,
    contract_trie_commit_ms: u64,
    class_trie_commit_ms: u64,
    read_stats_delta: ReadStats,
    rpc_read_fallback_count_delta: u64,
    rpc_read_fallback_total_ms_delta: u64,
    rpc_state_diff_count_delta: u64,
    rpc_state_diff_total_ms_delta: u64,
    bonsai_db_ops_delta: Option<BonsaiDbOpsDelta>,
}

#[derive(Serialize, Default, Clone)]
struct BonsaiDbOpsDelta {
    get_calls: [u64; 9],
    get_value_bytes: [u64; 9],
    insert_calls: [u64; 9],
    insert_old_value_present: [u64; 9],
    insert_old_value_bytes: [u64; 9],
    remove_calls: [u64; 9],
    remove_old_value_present: [u64; 9],
    remove_old_value_bytes: [u64; 9],
    contains_calls: [u64; 9],
    contains_hits: [u64; 9],
    iter_calls: [u64; 9],
    iter_keys: [u64; 9],
    iter_key_bytes: [u64; 9],
    iter_value_bytes: [u64; 9],
    write_batch_calls: u64,
    get_key_bytes: [u64; 9],
    insert_key_bytes: [u64; 9],
    insert_value_bytes: [u64; 9],
    remove_key_bytes: [u64; 9],
    contains_key_bytes: [u64; 9],
}

fn bonsai_db_ops_delta(after: BonsaiDbOpsSnapshot, before: BonsaiDbOpsSnapshot) -> BonsaiDbOpsDelta {
    fn sub_arr(a: [u64; 9], b: [u64; 9]) -> [u64; 9] {
        let mut out = [0u64; 9];
        for i in 0..9 {
            out[i] = a[i].saturating_sub(b[i]);
        }
        out
    }
    BonsaiDbOpsDelta {
        get_calls: sub_arr(after.get_calls, before.get_calls),
        get_value_bytes: sub_arr(after.get_value_bytes, before.get_value_bytes),
        insert_calls: sub_arr(after.insert_calls, before.insert_calls),
        insert_old_value_present: sub_arr(after.insert_old_value_present, before.insert_old_value_present),
        insert_old_value_bytes: sub_arr(after.insert_old_value_bytes, before.insert_old_value_bytes),
        remove_calls: sub_arr(after.remove_calls, before.remove_calls),
        remove_old_value_present: sub_arr(after.remove_old_value_present, before.remove_old_value_present),
        remove_old_value_bytes: sub_arr(after.remove_old_value_bytes, before.remove_old_value_bytes),
        contains_calls: sub_arr(after.contains_calls, before.contains_calls),
        contains_hits: sub_arr(after.contains_hits, before.contains_hits),
        iter_calls: sub_arr(after.iter_calls, before.iter_calls),
        iter_keys: sub_arr(after.iter_keys, before.iter_keys),
        iter_key_bytes: sub_arr(after.iter_key_bytes, before.iter_key_bytes),
        iter_value_bytes: sub_arr(after.iter_value_bytes, before.iter_value_bytes),
        write_batch_calls: after.write_batch_calls.saturating_sub(before.write_batch_calls),
        get_key_bytes: sub_arr(after.get_key_bytes, before.get_key_bytes),
        insert_key_bytes: sub_arr(after.insert_key_bytes, before.insert_key_bytes),
        insert_value_bytes: sub_arr(after.insert_value_bytes, before.insert_value_bytes),
        remove_key_bytes: sub_arr(after.remove_key_bytes, before.remove_key_bytes),
        contains_key_bytes: sub_arr(after.contains_key_bytes, before.contains_key_bytes),
    }
}

fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env()).init();

    let wall_clock_start = Instant::now();
    let args = Args::parse();
    bonsai_db_ops_set_enabled(args.bonsai_db_ops_metrics);

    prepare_output_dir(&args.out_dir, args.force)?;

    if args.synthetic {
        return run_synthetic(&args, wall_clock_start);
    }

    let db_path = resolve_db_path(&args.db_path)?;
    let source_db_path = resolve_db_path(args.source_db_path.as_deref().unwrap_or(&args.db_path))?;
    let rpc_url =
        args.rpc_url.clone().ok_or_else(|| anyhow!("--rpc-url is required unless running with --synthetic"))?;
    let rpc_stats = Arc::new(RpcStats::default());
    let rpc_client = BlockingClient::new();

    let target_storage = RocksDBStorage::open(&db_path, RocksDBConfig::default())
        .with_context(|| format!("Opening RocksDB at {}", db_path.display()))?;
    if args.source_read_only && source_db_path == db_path {
        bail!("--source-read-only requires a distinct --source-db-path");
    }

    let source_storage = if args.state_diff_source == StateDiffSource::Db {
        if source_db_path == db_path {
            target_storage.clone()
        } else if args.source_read_only {
            RocksDBStorage::open_read_only(&source_db_path, RocksDBConfig::default())
                .with_context(|| format!("Opening source RocksDB read-only at {}", source_db_path.display()))?
        } else {
            RocksDBStorage::open(&source_db_path, RocksDBConfig::default())
                .with_context(|| format!("Opening source RocksDB at {}", source_db_path.display()))?
        }
    } else {
        target_storage.clone()
    };
    let rayon_pool = rayon::ThreadPoolBuilder::new().build().context("Building rayon thread pool")?;

    let latest_applied = target_storage.get_latest_applied_trie_update()?.unwrap_or(0);
    let default_start = latest_applied.saturating_add(1);
    let start_block = args.start_block.unwrap_or(default_start);

    let end_block = match (args.end_block, args.block_count) {
        (Some(end), None) => end,
        (None, Some(count)) => {
            if count == 0 {
                bail!("block-count must be > 0");
            }
            start_block.checked_add(count - 1).context("block-count overflows end block")?
        }
        (None, None) => bail!("Either --end-block or --block-count must be provided"),
        _ => unreachable!("clap enforces conflicts"),
    };

    if args.state_diff_source == StateDiffSource::Db {
        let chain_tip = confirmed_chain_tip(&source_storage.get_chain_tip()?);
        if let Some(tip) = chain_tip {
            if end_block > tip {
                bail!("end_block {} is past chain tip {}", end_block, tip);
            }
        }
    }

    info!("Starting baseline apply: db={}, start={}, end={}", db_path.display(), start_block, end_block);

    let calls_path = args.out_dir.join("calls.jsonl");
    let roots_path = args.out_dir.join("roots.jsonl");
    let applies_path = args.out_dir.join("applies.jsonl");
    let summary_path = args.out_dir.join("run.json");

    if args.squash_range && args.apply_state_diff_columns {
        bail!("apply-state-diff-columns is not supported with --squash-range");
    }

    let compute_start = Instant::now();
    let mut squash_total = Duration::ZERO;
    let mut merklization_total = Duration::ZERO;

    let squashed_diff = if args.squash_range {
        let squash_start = Instant::now();
        let pre_range = args.pre_range_block.or_else(|| start_block.checked_sub(1));
        let diff = match args.state_diff_source {
            StateDiffSource::Db => Some(build_squashed_diff_db(&source_storage, start_block, end_block, pre_range)?),
            StateDiffSource::Rpc => Some(build_squashed_diff_rpc(
                &rpc_client,
                rpc_stats.clone(),
                &rpc_url,
                start_block,
                end_block,
                pre_range,
            )?),
        };
        squash_total = squash_start.elapsed();
        diff
    } else {
        None
    };

    let mut read_map = match &args.read_map {
        Some(path) => Some(ReadMap::load(path)?),
        None => None,
    };

    // For squashed applies, Bonsai may need nonce/class-hash reads at `end_block` for any leaf contract
    // touched anywhere in the squashed range. Baseline per-block read logs can miss these, so we
    // always prefill an end-block override map when we have DB access to the source.
    if args.squash_range && args.state_diff_source == StateDiffSource::Db {
        let diff = squashed_diff.as_ref().expect("squashed diff computed");
        let entries = build_override_map(diff, &source_storage, end_block)?;
        match &read_map {
            Some(map) => {
                for (key, value) in entries {
                    let mut guard = map.entries.write().expect("read map lock poisoned");
                    guard.entry(key).or_insert(value);
                }
            }
            None => {
                read_map = Some(ReadMap::from_entries(entries));
            }
        }
    }

    let read_map = if args.rpc_read_fallback && read_map.is_none() {
        Some(ReadMap::from_entries(HashMap::new()))
    } else {
        read_map
    };

    let read_map_writer = args.read_map_out.as_ref().map(|_| Arc::new(ReadMapWriter::new()));
    let rpc_fallback =
        if args.rpc_read_fallback { Some(RpcReadFallback::new(&rpc_url, rpc_stats.clone())?) } else { None };

    let recorder = Arc::new(JsonlReadRecorder::new(&calls_path, read_map, read_map_writer.clone(), rpc_fallback)?);
    set_read_hook(recorder.clone())?;

    let mut roots_writer = BufWriter::new(File::create(&roots_path)?);
    let mut applies_writer = BufWriter::new(File::create(&applies_path)?);

    let mut last_root: Option<Felt> = None;
    let mut apply_index: u64 = 0;
    if let Some(state_diff) = squashed_diff {
        set_context_block(Some(end_block));
        let stats_before = recorder.stats();
        let rpc_before = RpcSnapshot::capture(&rpc_stats);
        let bonsai_before = if args.bonsai_db_ops_metrics { Some(bonsai_db_ops_snapshot()) } else { None };
        let apply_start = Instant::now();
        let (state_root, timings) = apply_state_in_rayon(&rayon_pool, &target_storage, end_block, &state_diff)?;
        let apply_elapsed = apply_start.elapsed();
        let stats_after = recorder.stats();
        let rpc_after = RpcSnapshot::capture(&rpc_stats);
        let bonsai_after = if args.bonsai_db_ops_metrics { Some(bonsai_db_ops_snapshot()) } else { None };
        let bonsai_delta = match (bonsai_after, bonsai_before) {
            (Some(after), Some(before)) => Some(bonsai_db_ops_delta(after, before)),
            _ => None,
        };
        merklization_total += timings.total;
        last_root = Some(state_root);

        let diff_stats = state_diff_stats(&state_diff);
        let record = ApplyRecord {
            apply_index,
            apply_mode: "squash_range".to_string(),
            start_block,
            end_block,
            block_count: end_block - start_block + 1,
            touched_contracts: diff_stats.touched_contracts,
            storage_diffs_contracts: diff_stats.storage_diffs_contracts,
            storage_entries: diff_stats.storage_entries,
            unique_storage_keys: diff_stats.unique_storage_keys,
            max_storage_entries_per_contract: diff_stats.max_storage_entries_per_contract,
            nonce_updates: diff_stats.nonce_updates,
            deployed_contracts: diff_stats.deployed_contracts,
            replaced_classes: diff_stats.replaced_classes,
            declared_classes: diff_stats.declared_classes,
            migrated_compiled_classes: diff_stats.migrated_compiled_classes,
            deprecated_declared_classes: diff_stats.deprecated_declared_classes,
            apply_wall_clock_ms: apply_elapsed.as_millis() as u64,
            merklization_ms: timings.total.as_millis() as u64,
            contract_trie_root_ms: timings.contract_trie_root.as_millis() as u64,
            class_trie_root_ms: timings.class_trie_root.as_millis() as u64,
            contract_storage_commit_ms: timings.contract_trie.storage_commit.as_millis() as u64,
            contract_trie_commit_ms: timings.contract_trie.trie_commit.as_millis() as u64,
            class_trie_commit_ms: timings.class_trie.trie_commit.as_millis() as u64,
            read_stats_delta: read_stats_delta(stats_after, stats_before),
            rpc_read_fallback_count_delta: rpc_after.read_fallback_count.saturating_sub(rpc_before.read_fallback_count),
            rpc_read_fallback_total_ms_delta: rpc_after
                .read_fallback_total_ms
                .saturating_sub(rpc_before.read_fallback_total_ms),
            rpc_state_diff_count_delta: rpc_after.state_diff_count.saturating_sub(rpc_before.state_diff_count),
            rpc_state_diff_total_ms_delta: rpc_after.state_diff_total_ms.saturating_sub(rpc_before.state_diff_total_ms),
            bonsai_db_ops_delta: bonsai_delta,
        };
        serde_json::to_writer(&mut applies_writer, &record)?;
        applies_writer.write_all(b"\n")?;

        let record = RootRecord { block_number: end_block, state_root };
        serde_json::to_writer(&mut roots_writer, &record)?;
        roots_writer.write_all(b"\n")?;
    } else {
        for block_n in start_block..=end_block {
            set_context_block(Some(block_n));

            let state_diff = match args.state_diff_source {
                StateDiffSource::Db => source_storage
                    .get_block_state_diff(block_n)?
                    .with_context(|| format!("Missing state diff for block {}", block_n))?,
                StateDiffSource::Rpc => fetch_state_diff_rpc(&rpc_client, rpc_stats.clone(), &rpc_url, block_n)?,
            };

            if args.apply_state_diff_columns {
                target_storage.write_state_diff(block_n, &state_diff)?;
            }

            let stats_before = recorder.stats();
            let rpc_before = RpcSnapshot::capture(&rpc_stats);
            let bonsai_before = if args.bonsai_db_ops_metrics { Some(bonsai_db_ops_snapshot()) } else { None };
            let apply_start = Instant::now();
            let (state_root, timings) = apply_state_in_rayon(&rayon_pool, &target_storage, block_n, &state_diff)?;
            let apply_elapsed = apply_start.elapsed();
            let stats_after = recorder.stats();
            let rpc_after = RpcSnapshot::capture(&rpc_stats);
            let bonsai_after = if args.bonsai_db_ops_metrics { Some(bonsai_db_ops_snapshot()) } else { None };
            let bonsai_delta = match (bonsai_after, bonsai_before) {
                (Some(after), Some(before)) => Some(bonsai_db_ops_delta(after, before)),
                _ => None,
            };
            merklization_total += timings.total;
            last_root = Some(state_root);

            let diff_stats = state_diff_stats(&state_diff);
            let record = ApplyRecord {
                apply_index,
                apply_mode: "per_block".to_string(),
                start_block: block_n,
                end_block: block_n,
                block_count: 1,
                touched_contracts: diff_stats.touched_contracts,
                storage_diffs_contracts: diff_stats.storage_diffs_contracts,
                storage_entries: diff_stats.storage_entries,
                unique_storage_keys: diff_stats.unique_storage_keys,
                max_storage_entries_per_contract: diff_stats.max_storage_entries_per_contract,
                nonce_updates: diff_stats.nonce_updates,
                deployed_contracts: diff_stats.deployed_contracts,
                replaced_classes: diff_stats.replaced_classes,
                declared_classes: diff_stats.declared_classes,
                migrated_compiled_classes: diff_stats.migrated_compiled_classes,
                deprecated_declared_classes: diff_stats.deprecated_declared_classes,
                apply_wall_clock_ms: apply_elapsed.as_millis() as u64,
                merklization_ms: timings.total.as_millis() as u64,
                contract_trie_root_ms: timings.contract_trie_root.as_millis() as u64,
                class_trie_root_ms: timings.class_trie_root.as_millis() as u64,
                contract_storage_commit_ms: timings.contract_trie.storage_commit.as_millis() as u64,
                contract_trie_commit_ms: timings.contract_trie.trie_commit.as_millis() as u64,
                class_trie_commit_ms: timings.class_trie.trie_commit.as_millis() as u64,
                read_stats_delta: read_stats_delta(stats_after, stats_before),
                rpc_read_fallback_count_delta: rpc_after
                    .read_fallback_count
                    .saturating_sub(rpc_before.read_fallback_count),
                rpc_read_fallback_total_ms_delta: rpc_after
                    .read_fallback_total_ms
                    .saturating_sub(rpc_before.read_fallback_total_ms),
                rpc_state_diff_count_delta: rpc_after.state_diff_count.saturating_sub(rpc_before.state_diff_count),
                rpc_state_diff_total_ms_delta: rpc_after
                    .state_diff_total_ms
                    .saturating_sub(rpc_before.state_diff_total_ms),
                bonsai_db_ops_delta: bonsai_delta,
            };
            serde_json::to_writer(&mut applies_writer, &record)?;
            applies_writer.write_all(b"\n")?;
            apply_index += 1;

            let record = RootRecord { block_number: block_n, state_root };
            serde_json::to_writer(&mut roots_writer, &record)?;
            roots_writer.write_all(b"\n")?;
        }
    }

    roots_writer.flush()?;
    applies_writer.flush()?;
    recorder.flush()?;
    clear_read_hook();
    set_context_block(None);

    if let Some(writer) = &read_map_writer {
        if let Some(path) = &args.read_map_out {
            writer.write_to(path)?;
            if writer.mismatches() > 0 {
                warn!("Read map output had {} duplicate keys with mismatched values", writer.mismatches());
            }
        }
    }

    let Some(end_root) = last_root else {
        bail!("No blocks were processed; end root unavailable");
    };

    let compute_elapsed = compute_start.elapsed();

    let (rpc_end_root, rpc_root_check_ms) = if args.skip_rpc_root_check {
        (None, 0u64)
    } else {
        let start = Instant::now();
        let rpc_end_root = fetch_rpc_root(&rpc_client, &rpc_url, end_block)?;
        let elapsed = start.elapsed();
        let ms = elapsed.as_millis() as u64;
        if rpc_end_root != end_root {
            bail!("End block root mismatch at {}: db={}, rpc={}", end_block, end_root, rpc_end_root);
        }
        (Some(rpc_end_root), ms)
    };

    if args.require_read_map && !args.rpc_read_fallback && recorder.stats().override_misses > 0 {
        bail!("Read map misses detected: {} missing lookups", recorder.stats().override_misses);
    }

    let summary = RunSummary {
        db_path: db_path.display().to_string(),
        source_db_path: source_db_path.display().to_string(),
        source_read_only: args.source_read_only,
        state_diff_source: match args.state_diff_source {
            StateDiffSource::Db => "db".to_string(),
            StateDiffSource::Rpc => "rpc".to_string(),
        },
        rpc_url: rpc_url.to_string(),
        start_block,
        end_block,
        total_blocks: end_block - start_block + 1,
        end_block_state_root: end_root,
        rpc_end_block_state_root: rpc_end_root,
        wall_clock_total_ms: wall_clock_start.elapsed().as_millis() as u64,
        wall_clock_compute_ms: compute_elapsed.as_millis() as u64,
        rpc_root_check_ms,
        merklization_total_ms: merklization_total.as_millis() as u64,
        squash_total_ms: squash_total.as_millis() as u64,
        read_stats: recorder.stats(),
        rpc_read_fallback_count: rpc_stats.read_fallback_count.load(Ordering::Relaxed),
        rpc_read_fallback_total_ms: rpc_stats.read_fallback_total_ms.load(Ordering::Relaxed),
        rpc_state_diff_count: rpc_stats.state_diff_count.load(Ordering::Relaxed),
        rpc_state_diff_total_ms: rpc_stats.state_diff_total_ms.load(Ordering::Relaxed),
        bonsai_db_ops_metrics_enabled: args.bonsai_db_ops_metrics,
        bonsai_cf_names: BONSAI_CF_NAMES.iter().map(|name| (*name).to_string()).collect(),
        calls_path: calls_path.display().to_string(),
        roots_path: roots_path.display().to_string(),
        applies_path: applies_path.display().to_string(),
        read_map_path: args.read_map_out.as_ref().map(|p| p.display().to_string()),
    };

    let summary_file = File::create(&summary_path)?;
    serde_json::to_writer_pretty(summary_file, &summary)?;

    target_storage.flush()?;

    info!("Phase 1 completed: end block root matches RPC");
    Ok(())
}

fn resolve_db_path(input: &Path) -> Result<PathBuf> {
    if input.join("CURRENT").exists() {
        return Ok(input.to_path_buf());
    }
    let nested = input.join("db");
    if nested.join("CURRENT").exists() {
        return Ok(nested);
    }
    warn!("DB path does not look like a RocksDB directory: {}", input.display());
    Ok(input.to_path_buf())
}

fn prepare_output_dir(out_dir: &Path, force: bool) -> Result<()> {
    if out_dir.exists() {
        let calls = out_dir.join("calls.jsonl");
        let roots = out_dir.join("roots.jsonl");
        let summary = out_dir.join("run.json");
        if !force && (calls.exists() || roots.exists() || summary.exists()) {
            bail!("Output directory already contains results; use --force to overwrite: {}", out_dir.display());
        }
    }
    fs::create_dir_all(out_dir).with_context(|| format!("Creating output dir {}", out_dir.display()))?;
    Ok(())
}

fn confirmed_chain_tip(tip: &StorageChainTip) -> Option<u64> {
    match tip {
        StorageChainTip::Empty => None,
        StorageChainTip::Confirmed(n) => Some(*n),
        StorageChainTip::Preconfirmed { header, .. } => header.block_number.checked_sub(1),
    }
}

fn block_id(block_n: u64) -> serde_json::Value {
    serde_json::json!({ "block_number": block_n })
}

fn format_felt(value: Felt) -> String {
    format!("0x{:x}", value)
}

#[derive(Debug, serde::Deserialize)]
struct RpcError {
    code: i64,
    message: String,
}

#[derive(Debug, serde::Deserialize)]
struct RpcResponse<T> {
    result: Option<T>,
    error: Option<RpcError>,
}

fn is_contract_not_found(err: &RpcError) -> bool {
    if err.code == 20 {
        return true;
    }
    let msg = err.message.to_lowercase();
    msg.contains("contract not found") || msg.contains("contractnotfound")
}

fn rpc_call<T: DeserializeOwned>(
    client: &BlockingClient,
    rpc_url: &str,
    method: &str,
    params: serde_json::Value,
) -> Result<T> {
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
        "id": 1
    });

    let response = client
        .post(rpc_url)
        .json(&request)
        .send()
        .with_context(|| format!("RPC {method} request failed"))?
        .json::<RpcResponse<T>>()
        .with_context(|| format!("RPC {method} response parse failed"))?;

    if let Some(err) = response.error {
        return Err(anyhow!("RPC {method} error {}: {}", err.code, err.message));
    }

    response.result.ok_or_else(|| anyhow!("RPC {method} missing result"))
}

fn rpc_call_felt(
    client: &BlockingClient,
    rpc_url: &str,
    method: &str,
    params: serde_json::Value,
) -> Result<Option<Felt>> {
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
        "id": 1
    });

    let response = client
        .post(rpc_url)
        .json(&request)
        .send()
        .with_context(|| format!("RPC {method} request failed"))?
        .json::<RpcResponse<String>>()
        .with_context(|| format!("RPC {method} response parse failed"))?;

    if let Some(err) = response.error {
        if is_contract_not_found(&err) {
            return Ok(None);
        }
        return Err(anyhow!("RPC {method} error {}: {}", err.code, err.message));
    }

    let value = response.result.ok_or_else(|| anyhow!("RPC {method} missing result"))?;
    parse_felt_or_none(&value)
}

fn parse_felt_or_none(value: &str) -> Result<Option<Felt>> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let felt = Felt::from_hex_unchecked(trimmed);
    Ok(Some(felt))
}

fn fetch_state_diff_rpc(
    client: &BlockingClient,
    stats: Arc<RpcStats>,
    rpc_url: &str,
    block_n: u64,
) -> Result<StateDiff> {
    let start = Instant::now();
    let update = rpc_call::<MaybePreConfirmedStateUpdate>(
        client,
        rpc_url,
        "starknet_getStateUpdate",
        serde_json::json!([block_id(block_n)]),
    )?;
    let elapsed = start.elapsed();
    stats.state_diff_count.fetch_add(1, Ordering::Relaxed);
    stats.state_diff_total_ms.fetch_add(elapsed.as_millis() as u64, Ordering::Relaxed);

    match update {
        MaybePreConfirmedStateUpdate::Update(update) => Ok(rpc_state_update_to_internal(update)),
        MaybePreConfirmedStateUpdate::PreConfirmedUpdate(_) => {
            bail!("RPC returned pre-confirmed state update for block {}", block_n)
        }
    }
}

fn rpc_state_update_to_internal(update: RpcStateUpdate) -> StateDiff {
    rpc_state_diff_to_internal(update.state_diff)
}

fn rpc_state_diff_to_internal(diff: RpcStateDiff) -> StateDiff {
    let storage_diffs = diff
        .storage_diffs
        .into_iter()
        .map(|RpcContractStorageDiffItem { address, storage_entries }| ContractStorageDiffItem {
            address,
            storage_entries: storage_entries
                .into_iter()
                .map(|RpcStorageEntry { key, value }| StorageEntry { key, value })
                .collect(),
        })
        .collect();

    let declared_classes = diff
        .declared_classes
        .into_iter()
        .map(|RpcDeclaredClassItem { class_hash, compiled_class_hash }| DeclaredClassItem {
            class_hash,
            compiled_class_hash,
        })
        .collect();

    let deployed_contracts = diff
        .deployed_contracts
        .into_iter()
        .map(|RpcDeployedContractItem { address, class_hash }| DeployedContractItem { address, class_hash })
        .collect();

    let replaced_classes = diff
        .replaced_classes
        .into_iter()
        .map(|RpcReplacedClassItem { contract_address, class_hash }| ReplacedClassItem { contract_address, class_hash })
        .collect();

    let nonces = diff
        .nonces
        .into_iter()
        .map(|RpcNonceUpdate { contract_address, nonce }| NonceUpdate { contract_address, nonce })
        .collect();

    let mut converted = StateDiff {
        storage_diffs,
        deployed_contracts,
        declared_classes,
        old_declared_contracts: diff.deprecated_declared_classes,
        nonces,
        replaced_classes,
        migrated_compiled_classes: Vec::new(),
    };

    converted.sort();
    converted
}

fn fetch_rpc_root(client: &BlockingClient, rpc_url: &str, block_n: u64) -> Result<Felt> {
    let block = rpc_call::<MaybePreConfirmedBlockWithTxHashes>(
        client,
        rpc_url,
        "starknet_getBlockWithTxHashes",
        serde_json::json!([block_id(block_n)]),
    )
    .with_context(|| format!("RPC getBlockWithTxHashes failed for block {}", block_n))?;

    match block {
        MaybePreConfirmedBlockWithTxHashes::Block(block) => Ok(block.new_root),
        MaybePreConfirmedBlockWithTxHashes::PreConfirmedBlock(_) => {
            bail!("RPC returned pre-confirmed block for number {}", block_n)
        }
    }
}

fn apply_state_in_rayon(
    pool: &ThreadPool,
    storage: &RocksDBStorage,
    block_n: u64,
    state_diff: &mp_state_update::StateDiff,
) -> Result<(Felt, mc_db::rocksdb::global_trie::MerklizationTimings)> {
    pool.install(|| {
        let (root, timings) = storage.apply_to_global_trie(block_n, std::iter::once(state_diff))?;
        storage.write_latest_applied_trie_update(&Some(block_n))?;
        Ok((root, timings))
    })
}

#[derive(Default)]
struct StateDiffMap {
    storage_diffs: HashMap<Felt, HashMap<Felt, Felt>>,
    deployed_contracts: HashMap<Felt, Felt>,
    declared_classes: HashMap<Felt, Felt>,
    deprecated_declared_classes: std::collections::HashSet<Felt>,
    nonces: HashMap<Felt, Felt>,
    replaced_classes: HashMap<Felt, Felt>,
    touched_contracts: std::collections::HashSet<Felt>,
    migrated_compiled_classes: HashMap<Felt, Felt>,
}

impl StateDiffMap {
    fn apply_state_diff(&mut self, state_diff: &StateDiff) {
        for contract_diff in &state_diff.storage_diffs {
            let contract_addr = contract_diff.address;
            self.touched_contracts.insert(contract_addr);
            let contract_storage = self.storage_diffs.entry(contract_addr).or_default();
            for entry in &contract_diff.storage_entries {
                contract_storage.insert(entry.key, entry.value);
            }
        }

        for item in &state_diff.deployed_contracts {
            self.deployed_contracts.insert(item.address, item.class_hash);
        }
        for item in &state_diff.declared_classes {
            self.declared_classes.insert(item.class_hash, item.compiled_class_hash);
        }
        for item in &state_diff.nonces {
            self.nonces.insert(item.contract_address, item.nonce);
        }
        for item in &state_diff.replaced_classes {
            self.replaced_classes.insert(item.contract_address, item.class_hash);
        }
        for class_hash in &state_diff.old_declared_contracts {
            self.deprecated_declared_classes.insert(*class_hash);
        }
        for item in &state_diff.migrated_compiled_classes {
            self.migrated_compiled_classes.insert(item.class_hash, item.compiled_class_hash);
        }
    }

    fn to_raw_state_diff(&self) -> StateDiff {
        let storage_diffs: Vec<ContractStorageDiffItem> = self
            .touched_contracts
            .iter()
            .filter_map(|contract_addr| {
                self.storage_diffs.get(contract_addr).map(|storage_map| {
                    let storage_entries: Vec<StorageEntry> =
                        storage_map.iter().map(|(key, value)| StorageEntry { key: *key, value: *value }).collect();
                    ContractStorageDiffItem { address: *contract_addr, storage_entries }
                })
            })
            .filter(|item| !item.storage_entries.is_empty())
            .collect();

        let deployed_contracts = self
            .deployed_contracts
            .iter()
            .map(|(address, class_hash)| DeployedContractItem { address: *address, class_hash: *class_hash })
            .collect();

        let declared_classes = self
            .declared_classes
            .iter()
            .map(|(class_hash, compiled_class_hash)| DeclaredClassItem {
                class_hash: *class_hash,
                compiled_class_hash: *compiled_class_hash,
            })
            .collect();

        let nonces = self
            .nonces
            .iter()
            .map(|(contract_address, nonce)| NonceUpdate { contract_address: *contract_address, nonce: *nonce })
            .collect();

        let replaced_classes = self
            .replaced_classes
            .iter()
            .map(|(contract_address, class_hash)| ReplacedClassItem {
                contract_address: *contract_address,
                class_hash: *class_hash,
            })
            .collect();

        let deprecated_declared_classes = self.deprecated_declared_classes.iter().copied().collect();

        let migrated_compiled_classes = self
            .migrated_compiled_classes
            .iter()
            .map(|(class_hash, compiled_class_hash)| MigratedClassItem {
                class_hash: *class_hash,
                compiled_class_hash: *compiled_class_hash,
            })
            .collect();

        let mut diff = StateDiff {
            storage_diffs,
            deployed_contracts,
            declared_classes,
            old_declared_contracts: deprecated_declared_classes,
            nonces,
            replaced_classes,
            migrated_compiled_classes,
        };
        diff.sort();
        diff
    }
}

fn build_squashed_diff_db(
    source: &RocksDBStorage,
    start_block: u64,
    end_block: u64,
    pre_range_block: Option<u64>,
) -> Result<StateDiff> {
    let mut map = StateDiffMap::default();
    for block_n in start_block..=end_block {
        let state_diff = source
            .get_block_state_diff(block_n)?
            .with_context(|| format!("Missing state diff for block {}", block_n))?;
        map.apply_state_diff(&state_diff);
    }

    let raw = map.to_raw_state_diff();
    let mut compressed = compress_state_diff_sync(raw, pre_range_block, source)?;
    compressed.sort();
    Ok(compressed)
}

fn build_squashed_diff_rpc(
    client: &BlockingClient,
    stats: Arc<RpcStats>,
    rpc_url: &str,
    start_block: u64,
    end_block: u64,
    _pre_range_block: Option<u64>,
) -> Result<StateDiff> {
    let mut map = StateDiffMap::default();
    for block_n in start_block..=end_block {
        let state_diff = fetch_state_diff_rpc(client, stats.clone(), rpc_url, block_n)?;
        map.apply_state_diff(&state_diff);
    }

    let raw = map.to_raw_state_diff();
    let mut compressed = compress_state_diff_no_db(raw);
    compressed.sort();
    Ok(compressed)
}

fn compress_state_diff_sync(
    raw_state_diff: StateDiff,
    pre_range_block: Option<u64>,
    source: &RocksDBStorage,
) -> Result<StateDiff> {
    let mut storage_diffs: Vec<ContractStorageDiffItem> = Vec::new();
    for contract_diff in raw_state_diff.storage_diffs {
        let ContractStorageDiffItem { address, storage_entries } = contract_diff;
        let filtered_entries = match pre_range_block {
            Some(block_n) => {
                let contract_existed = source.get_contract_class_hash_at(block_n, &address)?.is_some();
                if contract_existed {
                    let mut entries = Vec::new();
                    for entry in storage_entries {
                        let pre_value = source.get_storage_at(block_n, &address, &entry.key).ok().flatten();
                        if pre_value != Some(entry.value) {
                            entries.push(entry);
                        }
                    }
                    entries
                } else {
                    storage_entries.into_iter().filter(|entry| entry.value != Felt::ZERO).collect()
                }
            }
            None => storage_entries.into_iter().filter(|entry| entry.value != Felt::ZERO).collect(),
        };

        if !filtered_entries.is_empty() {
            storage_diffs.push(ContractStorageDiffItem { address, storage_entries: filtered_entries });
        }
    }

    let mut deployed_contracts_map: HashMap<Felt, Felt> =
        raw_state_diff.deployed_contracts.into_iter().map(|item| (item.address, item.class_hash)).collect();
    let mut replaced_classes_map: HashMap<Felt, Felt> =
        raw_state_diff.replaced_classes.into_iter().map(|item| (item.contract_address, item.class_hash)).collect();

    replaced_classes_map.retain(|contract_address, class_hash| {
        match deployed_contracts_map.get_mut(contract_address) {
            Some(existing_class_hash) => {
                *existing_class_hash = *class_hash;
                false
            }
            None => true,
        }
    });

    let replaced_classes = match pre_range_block {
        Some(block_n) => {
            let mut filtered = Vec::new();
            for (contract_address, class_hash) in replaced_classes_map {
                match source.get_contract_class_hash_at(block_n, &contract_address)? {
                    Some(prev_class_hash) if prev_class_hash == class_hash => {}
                    _ => filtered.push(ReplacedClassItem { contract_address, class_hash }),
                }
            }
            filtered
        }
        None => replaced_classes_map
            .into_iter()
            .map(|(contract_address, class_hash)| ReplacedClassItem { contract_address, class_hash })
            .collect(),
    };

    let deployed_contracts = deployed_contracts_map
        .into_iter()
        .map(|(address, class_hash)| DeployedContractItem { address, class_hash })
        .collect();

    let mut compressed = StateDiff {
        storage_diffs,
        deployed_contracts,
        declared_classes: raw_state_diff.declared_classes,
        old_declared_contracts: raw_state_diff.old_declared_contracts,
        nonces: raw_state_diff.nonces,
        replaced_classes,
        migrated_compiled_classes: raw_state_diff.migrated_compiled_classes,
    };

    compressed.sort();
    Ok(compressed)
}

fn compress_state_diff_no_db(raw_state_diff: StateDiff) -> StateDiff {
    // Without a DB view of the pre-range state we cannot safely elide zero-valued updates.
    // A zero can represent a meaningful deletion (pre != 0, post == 0), so keep them for correctness.
    let StateDiff {
        storage_diffs,
        deployed_contracts,
        declared_classes,
        old_declared_contracts,
        nonces,
        replaced_classes,
        migrated_compiled_classes,
    } = raw_state_diff;

    let mut deployed_contracts_map: HashMap<Felt, Felt> =
        deployed_contracts.into_iter().map(|item| (item.address, item.class_hash)).collect();
    let mut replaced_classes_map: HashMap<Felt, Felt> =
        replaced_classes.into_iter().map(|item| (item.contract_address, item.class_hash)).collect();

    replaced_classes_map.retain(|contract_address, class_hash| {
        match deployed_contracts_map.get_mut(contract_address) {
            Some(existing_class_hash) => {
                *existing_class_hash = *class_hash;
                false
            }
            None => true,
        }
    });

    let replaced_classes = replaced_classes_map
        .into_iter()
        .map(|(contract_address, class_hash)| ReplacedClassItem { contract_address, class_hash })
        .collect();

    let deployed_contracts = deployed_contracts_map
        .into_iter()
        .map(|(address, class_hash)| DeployedContractItem { address, class_hash })
        .collect();

    let mut compressed = StateDiff {
        storage_diffs,
        deployed_contracts,
        declared_classes,
        old_declared_contracts,
        nonces,
        replaced_classes,
        migrated_compiled_classes,
    };

    compressed.sort();
    compressed
}

fn build_override_map(
    diff: &StateDiff,
    source: &RocksDBStorage,
    block_n: u64,
) -> Result<HashMap<String, Option<Felt>>> {
    let mut leaf_contracts: std::collections::HashSet<Felt> = std::collections::HashSet::new();
    for contract_diff in &diff.storage_diffs {
        leaf_contracts.insert(contract_diff.address);
    }
    for item in &diff.nonces {
        leaf_contracts.insert(item.contract_address);
    }
    for item in &diff.deployed_contracts {
        leaf_contracts.insert(item.address);
    }
    for item in &diff.replaced_classes {
        leaf_contracts.insert(item.contract_address);
    }

    let mut entries: HashMap<String, Option<Felt>> = HashMap::new();
    for address in leaf_contracts {
        let nonce_value = source.get_contract_nonce_at(block_n, &address).ok().flatten();
        let nonce_op = ReadOp::GetContractNonceAt { block_n, contract_address: address };
        let nonce_key = serde_json::to_string(&nonce_op).context("Serializing nonce ReadOp")?;
        entries.insert(nonce_key, nonce_value);

        let class_value = source.get_contract_class_hash_at(block_n, &address).ok().flatten();
        let class_op = ReadOp::GetContractClassHashAt { block_n, contract_address: address };
        let class_key = serde_json::to_string(&class_op).context("Serializing class hash ReadOp")?;
        entries.insert(class_key, class_value);
    }

    Ok(entries)
}

// ================================================================================================
// Synthetic micro-benchmark
// ================================================================================================

/// Coefficients for the single-block warm-worker estimator captured in IMP.md.
/// Used only for validating the model on synthetic workloads.
const EQ_INTERCEPT: f64 = -2.9902135658816897;
const EQ_UNIQUE_STORAGE_KEYS: f64 = 0.057034857128496035;
const EQ_TOUCHED_CONTRACTS: f64 = 1.92242587519791;
const EQ_DEPLOYED_CONTRACTS: f64 = 12.900142243220861;

fn eq_predict_ms(unique_storage_keys: u64, touched_contracts: u64, deployed_contracts: u64) -> f64 {
    let raw = EQ_INTERCEPT
        + EQ_UNIQUE_STORAGE_KEYS * (unique_storage_keys as f64)
        + EQ_TOUCHED_CONTRACTS * (touched_contracts as f64)
        + EQ_DEPLOYED_CONTRACTS * (deployed_contracts as f64);
    raw.max(0.0)
}

#[derive(Serialize)]
struct SyntheticApplyRecord {
    apply_index: u64,
    block_n: u64,
    touched_contracts: u64,
    unique_storage_keys: u64,
    deployed_contracts: u64,
    merklization_ms: u64,
    contract_storage_commit_ms: u64,
    contract_trie_commit_ms: u64,
    predicted_merklization_ms: f64,
    prediction_error_ms: f64,
    read_stats_delta: ReadStats,
    bonsai_db_ops_delta: Option<BonsaiDbOpsDelta>,
}

#[derive(Serialize)]
struct SyntheticRunSummary {
    db_path: String,
    out_dir: String,
    synthetic_contracts: u64,
    synthetic_storage_keys_per_contract: u64,
    synthetic_deployed_contracts: u64,
    synthetic_nonce_only_contracts: u64,
    synthetic_key_pattern: String,
    synthetic_fixed_shape: bool,
    synthetic_reuse_db: bool,
    synthetic_start_block: u64,
    warmup_iters: u64,
    iters: u64,
    /// Total wall-clock time for the run (including warmup + setup).
    wall_clock_total_ms: u64,
    /// Sum of per-apply merkle timings across recorded iterations (warmup excluded).
    merklization_total_ms: u64,
    /// Prediction error stats over recorded iterations.
    pred_mae_ms: f64,
    pred_rmse_ms: f64,
    pred_err_min_ms: f64,
    pred_err_p50_ms: f64,
    pred_err_p90_ms: f64,
    pred_err_max_ms: f64,
    calls_path: String,
    applies_path: String,
}

#[derive(Clone, Copy)]
struct XorShift64 {
    state: u64,
}

impl XorShift64 {
    fn new(seed: u64) -> Self {
        Self { state: if seed == 0 { 0x9e3779b97f4a7c15 } else { seed } }
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }

    fn fill_bytes(&mut self, out: &mut [u8]) {
        let mut i = 0usize;
        while i < out.len() {
            let v = self.next_u64().to_be_bytes();
            let n = (out.len() - i).min(8);
            out[i..i + n].copy_from_slice(&v[..n]);
            i += n;
        }
    }

    fn next_felt251(&mut self) -> Felt {
        let mut bytes = [0u8; 32];
        self.fill_bytes(&mut bytes);
        // Ensure value < 2^251 < Starknet modulus.
        bytes[0] &= 0b0000_0111;
        Felt::from_bytes_be(&bytes)
    }
}

struct SyntheticReadHook {
    writer: Mutex<BufWriter<File>>,
    counters: ReadCounters,
    nonce_by_contract: HashMap<Felt, Felt>,
    class_hash_by_contract: HashMap<Felt, Felt>,
    storage_by_key: HashMap<(Felt, Felt), Felt>,
}

impl SyntheticReadHook {
    fn new(
        path: &Path,
        nonce_by_contract: HashMap<Felt, Felt>,
        class_hash_by_contract: HashMap<Felt, Felt>,
        storage_by_key: HashMap<(Felt, Felt), Felt>,
    ) -> Result<Self> {
        let file = File::create(path).with_context(|| format!("Creating read log at {}", path.display()))?;
        Ok(Self {
            writer: Mutex::new(BufWriter::new(file)),
            counters: ReadCounters::default(),
            nonce_by_contract,
            class_hash_by_contract,
            storage_by_key,
        })
    }

    fn stats(&self) -> ReadStats {
        ReadStats {
            total: self.counters.total.load(Ordering::Relaxed),
            storage_at: self.counters.storage_at.load(Ordering::Relaxed),
            nonce_at: self.counters.nonce_at.load(Ordering::Relaxed),
            class_hash_at: self.counters.class_hash_at.load(Ordering::Relaxed),
            override_hits: self.counters.override_hits.load(Ordering::Relaxed),
            override_misses: self.counters.override_misses.load(Ordering::Relaxed),
        }
    }
}

impl ReadHook for SyntheticReadHook {
    fn override_read(&self, op: &ReadOp) -> Option<Option<Felt>> {
        match op {
            ReadOp::GetStorageAt { contract_address, key, .. } => {
                if let Some(v) = self.storage_by_key.get(&(*contract_address, *key)) {
                    self.counters.override_hits.fetch_add(1, Ordering::Relaxed);
                    Some(Some(*v))
                } else {
                    self.counters.override_misses.fetch_add(1, Ordering::Relaxed);
                    // Avoid DB fallback in synthetic mode.
                    Some(Some(Felt::ZERO))
                }
            }
            ReadOp::GetContractNonceAt { contract_address, .. } => {
                if let Some(v) = self.nonce_by_contract.get(contract_address) {
                    self.counters.override_hits.fetch_add(1, Ordering::Relaxed);
                    Some(Some(*v))
                } else {
                    self.counters.override_misses.fetch_add(1, Ordering::Relaxed);
                    Some(Some(Felt::ZERO))
                }
            }
            ReadOp::GetContractClassHashAt { contract_address, .. } => {
                if let Some(v) = self.class_hash_by_contract.get(contract_address) {
                    self.counters.override_hits.fetch_add(1, Ordering::Relaxed);
                    Some(Some(*v))
                } else {
                    self.counters.override_misses.fetch_add(1, Ordering::Relaxed);
                    Some(Some(Felt::ZERO))
                }
            }
        }
    }

    fn on_read(&self, event: ReadEvent) {
        self.counters.total.fetch_add(1, Ordering::Relaxed);
        match event.op {
            ReadOp::GetStorageAt { .. } => {
                self.counters.storage_at.fetch_add(1, Ordering::Relaxed);
            }
            ReadOp::GetContractNonceAt { .. } => {
                self.counters.nonce_at.fetch_add(1, Ordering::Relaxed);
            }
            ReadOp::GetContractClassHashAt { .. } => {
                self.counters.class_hash_at.fetch_add(1, Ordering::Relaxed);
            }
        }

        let mut guard = self.writer.lock().expect("read log lock poisoned");
        if let Err(err) = serde_json::to_writer(&mut *guard, &event) {
            panic!("Failed to write read event JSON: {err}");
        }
        if let Err(err) = guard.write_all(b"\n") {
            panic!("Failed to write read event newline: {err}");
        }
    }
}

fn run_synthetic(args: &Args, wall_clock_start: Instant) -> Result<()> {
    let db_path = resolve_db_path(&args.db_path)?;

    if !args.synthetic_reuse_db {
        // Fresh synthetic DB: require an empty directory (or --force to wipe it).
        if db_path.exists() {
            let has_entries = fs::read_dir(&db_path).map(|mut it| it.next().is_some()).unwrap_or(false);
            if has_entries {
                if args.force {
                    fs::remove_dir_all(&db_path)
                        .with_context(|| format!("Removing synthetic db dir {}", db_path.display()))?;
                } else {
                    bail!(
                        "Synthetic db path {} is not empty; pass --force to delete it (or pass --synthetic-reuse-db)",
                        db_path.display()
                    );
                }
            }
        }
        fs::create_dir_all(&db_path).with_context(|| format!("Creating synthetic db dir {}", db_path.display()))?;
    } else if !db_path.join("CURRENT").exists() {
        bail!("--synthetic-reuse-db requires a RocksDB directory (missing CURRENT): {}", db_path.display());
    }

    let target_storage = RocksDBStorage::open(&db_path, RocksDBConfig::default())
        .with_context(|| format!("Opening RocksDB at {}", db_path.display()))?;
    let rayon_pool = rayon::ThreadPoolBuilder::new().build().context("Building rayon thread pool")?;

    let calls_path = args.out_dir.join("calls.jsonl");
    let applies_path = args.out_dir.join("applies.jsonl");
    let summary_path = args.out_dir.join("run.json");

    let mut rng = XorShift64::new(args.synthetic_seed);
    let contracts_n = args.synthetic_contracts as usize;
    let keys_per_contract_n = args.synthetic_storage_keys_per_contract as usize;
    let deploy_n = args.synthetic_deployed_contracts as usize;
    let nonce_only_n = args.synthetic_nonce_only_contracts as usize;

    // Fixed shape inputs (addresses + keys) so that per-apply variance is dominated by the updates, not by trie growth.
    let mut contract_addrs: Vec<Felt> = Vec::with_capacity(contracts_n);
    for _ in 0..contracts_n {
        contract_addrs.push(rng.next_felt251());
    }
    let mut deployed_addrs: Vec<Felt> = Vec::with_capacity(deploy_n);
    for _ in 0..deploy_n {
        deployed_addrs.push(rng.next_felt251());
    }
    let mut nonce_only_addrs: Vec<Felt> = Vec::with_capacity(nonce_only_n);
    for _ in 0..nonce_only_n {
        nonce_only_addrs.push(rng.next_felt251());
    }

    // Ensure deployed addrs don't overlap contract addrs.
    let mut used: std::collections::HashSet<Felt> = contract_addrs.iter().copied().collect();
    for addr in &mut deployed_addrs {
        while used.contains(addr) {
            *addr = rng.next_felt251();
        }
        used.insert(*addr);
    }
    for addr in &mut nonce_only_addrs {
        while used.contains(addr) {
            *addr = rng.next_felt251();
        }
        used.insert(*addr);
    }

    let mut storage_keys_by_contract: Vec<Vec<Felt>> = Vec::with_capacity(contracts_n);
    for _ in 0..contracts_n {
        let mut keys: Vec<Felt> = Vec::with_capacity(keys_per_contract_n);
        match args.synthetic_key_pattern {
            SyntheticKeyPattern::Random => {
                for _ in 0..keys_per_contract_n {
                    keys.push(rng.next_felt251());
                }
            }
            SyntheticKeyPattern::CommonPrefix => {
                let mut base = [0u8; 32];
                rng.fill_bytes(&mut base);
                base[0] &= 0b0000_0111;
                // Keep first 24 bytes fixed; vary last 8 bytes.
                for ki in 0..keys_per_contract_n {
                    let mut b = base;
                    b[24..].copy_from_slice(&(ki as u64).to_be_bytes());
                    keys.push(Felt::from_bytes_be(&b));
                }
            }
            SyntheticKeyPattern::Sequential => {
                for ki in 0..keys_per_contract_n {
                    let mut b = [0u8; 32];
                    b[24..].copy_from_slice(&((1 + ki) as u64).to_be_bytes());
                    b[0] &= 0b0000_0111;
                    keys.push(Felt::from_bytes_be(&b));
                }
            }
        }
        keys.sort();
        keys.dedup();
        while keys.len() < keys_per_contract_n {
            keys.push(rng.next_felt251());
            keys.sort();
            keys.dedup();
        }
        storage_keys_by_contract.push(keys);
    }

    // Override maps for nonce/class-hash reads (kept constant).
    let mut nonce_by_contract: HashMap<Felt, Felt> = HashMap::new();
    let mut class_hash_by_contract: HashMap<Felt, Felt> = HashMap::new();
    for addr in contract_addrs.iter().chain(deployed_addrs.iter()).chain(nonce_only_addrs.iter()) {
        nonce_by_contract.insert(*addr, Felt::ZERO);
        class_hash_by_contract.insert(*addr, Felt::from_hex_unchecked("0x1"));
    }

    let hook =
        Arc::new(SyntheticReadHook::new(&calls_path, nonce_by_contract, class_hash_by_contract, HashMap::new())?);
    set_read_hook(hook.clone())?;

    let mut applies_writer = BufWriter::new(File::create(&applies_path)?);
    let mut merklization_total = Duration::ZERO;
    let mut pred_errs: Vec<f64> = Vec::new();

    let start_block = match args.synthetic_start_block {
        Some(n) => n,
        None if args.synthetic_reuse_db => {
            target_storage.get_latest_applied_trie_update()?.unwrap_or(0).saturating_add(1)
        }
        None => 1,
    };

    let total_iters = args.synthetic_warmup_iters + args.synthetic_iters;
    for i in 0..total_iters {
        let block_n = start_block + i;
        set_context_block(Some(block_n));

        let mut state_diff = StateDiff {
            storage_diffs: Vec::with_capacity(contracts_n),
            deployed_contracts: Vec::with_capacity(deploy_n),
            declared_classes: Vec::new(),
            old_declared_contracts: Vec::new(),
            nonces: Vec::new(),
            replaced_classes: Vec::new(),
            migrated_compiled_classes: Vec::new(),
        };

        for (addr, keys) in contract_addrs.iter().zip(storage_keys_by_contract.iter()) {
            let mut storage_entries: Vec<StorageEntry> = Vec::with_capacity(keys_per_contract_n);
            for key in keys {
                let mut value = rng.next_felt251();
                if value == Felt::ZERO {
                    value = Felt::from_hex_unchecked("0x1");
                }
                storage_entries.push(StorageEntry { key: *key, value });
            }
            state_diff.storage_diffs.push(ContractStorageDiffItem { address: *addr, storage_entries });
        }

        for addr in &deployed_addrs {
            state_diff
                .deployed_contracts
                .push(DeployedContractItem { address: *addr, class_hash: Felt::from_hex_unchecked("0x1") });
        }

        for (idx, addr) in nonce_only_addrs.iter().enumerate() {
            // Vary the nonce per block so the leaf hash is not constant.
            let mut b = [0u8; 32];
            let v = ((block_n as u64) << 32) | (idx as u64);
            b[24..].copy_from_slice(&v.to_be_bytes());
            b[0] &= 0b0000_0111;
            let nonce = Felt::from_bytes_be(&b);
            state_diff.nonces.push(NonceUpdate { contract_address: *addr, nonce });
        }

        state_diff.sort();

        let stats_before = hook.stats();
        let bonsai_before = if args.bonsai_db_ops_metrics { Some(bonsai_db_ops_snapshot()) } else { None };
        let (_state_root, timings) = apply_state_in_rayon(&rayon_pool, &target_storage, block_n, &state_diff)?;
        let stats_after = hook.stats();
        let bonsai_after = if args.bonsai_db_ops_metrics { Some(bonsai_db_ops_snapshot()) } else { None };
        let bonsai_delta = match (bonsai_after, bonsai_before) {
            (Some(after), Some(before)) => Some(bonsai_db_ops_delta(after, before)),
            _ => None,
        };

        if i >= args.synthetic_warmup_iters {
            merklization_total += timings.total;
            let diff_stats = state_diff_stats(&state_diff);
            let pred = eq_predict_ms(
                diff_stats.unique_storage_keys,
                diff_stats.touched_contracts,
                diff_stats.deployed_contracts,
            );
            let measured = timings.total.as_millis() as f64;
            let err = measured - pred;
            pred_errs.push(err);

            let record = SyntheticApplyRecord {
                apply_index: i - args.synthetic_warmup_iters,
                block_n,
                touched_contracts: diff_stats.touched_contracts,
                unique_storage_keys: diff_stats.unique_storage_keys,
                deployed_contracts: diff_stats.deployed_contracts,
                merklization_ms: timings.total.as_millis() as u64,
                contract_storage_commit_ms: timings.contract_trie.storage_commit.as_millis() as u64,
                contract_trie_commit_ms: timings.contract_trie.trie_commit.as_millis() as u64,
                predicted_merklization_ms: pred,
                prediction_error_ms: err,
                read_stats_delta: read_stats_delta(stats_after, stats_before),
                bonsai_db_ops_delta: bonsai_delta,
            };
            serde_json::to_writer(&mut applies_writer, &record)?;
            applies_writer.write_all(b"\n")?;
        }

        if !args.synthetic_fixed_shape {
            // Keep addresses fixed but randomize keys to probe a wider variety of trie shapes.
            if args.synthetic_key_pattern == SyntheticKeyPattern::Random {
                for keys in &mut storage_keys_by_contract {
                    for k in keys.iter_mut() {
                        *k = rng.next_felt251();
                    }
                    keys.sort();
                    keys.dedup();
                    while keys.len() < keys_per_contract_n {
                        keys.push(rng.next_felt251());
                        keys.sort();
                        keys.dedup();
                    }
                }
            }
        }
    }

    applies_writer.flush()?;
    clear_read_hook();
    set_context_block(None);

    pred_errs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mae = pred_errs.iter().map(|e| e.abs()).sum::<f64>() / (pred_errs.len().max(1) as f64);
    let rmse = (pred_errs.iter().map(|e| e * e).sum::<f64>() / (pred_errs.len().max(1) as f64)).sqrt();
    let p50 = if pred_errs.is_empty() { 0.0 } else { pred_errs[pred_errs.len() / 2] };
    let p90 = if pred_errs.is_empty() {
        0.0
    } else {
        pred_errs[((pred_errs.len() as f64) * 0.9).floor().min((pred_errs.len() - 1) as f64) as usize]
    };
    let min = pred_errs.first().copied().unwrap_or(0.0);
    let max = pred_errs.last().copied().unwrap_or(0.0);

    let summary = SyntheticRunSummary {
        db_path: db_path.display().to_string(),
        out_dir: args.out_dir.display().to_string(),
        synthetic_contracts: args.synthetic_contracts,
        synthetic_storage_keys_per_contract: args.synthetic_storage_keys_per_contract,
        synthetic_deployed_contracts: args.synthetic_deployed_contracts,
        synthetic_nonce_only_contracts: args.synthetic_nonce_only_contracts,
        synthetic_key_pattern: format!("{:?}", args.synthetic_key_pattern),
        synthetic_fixed_shape: args.synthetic_fixed_shape,
        synthetic_reuse_db: args.synthetic_reuse_db,
        synthetic_start_block: start_block,
        warmup_iters: args.synthetic_warmup_iters,
        iters: args.synthetic_iters,
        wall_clock_total_ms: wall_clock_start.elapsed().as_millis() as u64,
        merklization_total_ms: merklization_total.as_millis() as u64,
        pred_mae_ms: mae,
        pred_rmse_ms: rmse,
        pred_err_min_ms: min,
        pred_err_p50_ms: p50,
        pred_err_p90_ms: p90,
        pred_err_max_ms: max,
        calls_path: calls_path.display().to_string(),
        applies_path: applies_path.display().to_string(),
    };

    let summary_file = File::create(&summary_path)?;
    serde_json::to_writer_pretty(summary_file, &summary)?;

    info!(
        "Synthetic run complete: merklization_total_ms={} pred_mae_ms={:.3} pred_rmse_ms={:.3}",
        summary.merklization_total_ms, summary.pred_mae_ms, summary.pred_rmse_ms
    );
    Ok(())
}
