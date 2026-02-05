use anyhow::{anyhow, bail, Context, Result};
use clap::Parser;
use mc_db::read_hook::{clear_read_hook, set_context_block, set_read_hook, ReadEvent, ReadHook, ReadOp};
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
    #[arg(long, env = "PARALLEL_MERKLELIZATION_RPC_URL", value_name = "URL")]
    rpc_url: String,
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
    #[arg(long)]
    force: bool,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum, PartialEq, Eq)]
enum StateDiffSource {
    Db,
    Rpc,
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
    rpc_end_block_state_root: Felt,
    read_stats: ReadStats,
    rpc_read_fallback_count: u64,
    rpc_read_fallback_total_ms: u64,
    rpc_state_diff_count: u64,
    rpc_state_diff_total_ms: u64,
    calls_path: String,
    roots_path: String,
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

fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env()).init();

    let args = Args::parse();
    let db_path = resolve_db_path(&args.db_path)?;
    let source_db_path = resolve_db_path(args.source_db_path.as_deref().unwrap_or(&args.db_path))?;
    let rpc_url = args.rpc_url.clone();
    let rpc_stats = Arc::new(RpcStats::default());
    let rpc_client = BlockingClient::new();

    prepare_output_dir(&args.out_dir, args.force)?;

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
    let summary_path = args.out_dir.join("run.json");

    if args.squash_range && args.apply_state_diff_columns {
        bail!("apply-state-diff-columns is not supported with --squash-range");
    }

    let squashed_diff = if args.squash_range {
        let pre_range = args.pre_range_block.or_else(|| start_block.checked_sub(1));
        match args.state_diff_source {
            StateDiffSource::Db => Some(build_squashed_diff_db(&source_storage, start_block, end_block, pre_range)?),
            StateDiffSource::Rpc => Some(build_squashed_diff_rpc(
                &rpc_client,
                rpc_stats.clone(),
                &args.rpc_url,
                start_block,
                end_block,
                pre_range,
            )?),
        }
    } else {
        None
    };

    let generated_read_map =
        if args.squash_range && args.read_map.is_none() && args.state_diff_source == StateDiffSource::Db {
            let diff = squashed_diff.as_ref().expect("squashed diff computed");
            Some(build_override_map(diff, &source_storage, end_block)?)
        } else {
            None
        };

    let read_map = match (&args.read_map, generated_read_map) {
        (Some(path), _) => Some(ReadMap::load(path)?),
        (None, Some(entries)) => Some(ReadMap::from_entries(entries)),
        (None, None) => None,
    };

    let read_map = if args.rpc_read_fallback && read_map.is_none() {
        Some(ReadMap::from_entries(HashMap::new()))
    } else {
        read_map
    };

    let read_map_writer = args.read_map_out.as_ref().map(|_| Arc::new(ReadMapWriter::new()));
    let rpc_fallback =
        if args.rpc_read_fallback { Some(RpcReadFallback::new(&args.rpc_url, rpc_stats.clone())?) } else { None };

    let recorder = Arc::new(JsonlReadRecorder::new(&calls_path, read_map, read_map_writer.clone(), rpc_fallback)?);
    set_read_hook(recorder.clone())?;

    let mut roots_writer = BufWriter::new(File::create(&roots_path)?);

    let mut last_root: Option<Felt> = None;
    if let Some(state_diff) = squashed_diff {
        set_context_block(Some(end_block));
        let (state_root, _timings) = apply_state_in_rayon(&rayon_pool, &target_storage, end_block, &state_diff)?;
        last_root = Some(state_root);
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
                StateDiffSource::Rpc => fetch_state_diff_rpc(&rpc_client, rpc_stats.clone(), &args.rpc_url, block_n)?,
            };

            if args.apply_state_diff_columns {
                target_storage.write_state_diff(block_n, &state_diff)?;
            }

            let (state_root, _timings) = apply_state_in_rayon(&rayon_pool, &target_storage, block_n, &state_diff)?;
            last_root = Some(state_root);

            let record = RootRecord { block_number: block_n, state_root };
            serde_json::to_writer(&mut roots_writer, &record)?;
            roots_writer.write_all(b"\n")?;
        }
    }

    roots_writer.flush()?;
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

    let rpc_end_root = fetch_rpc_root(&rpc_client, &rpc_url, end_block)?;
    if rpc_end_root != end_root {
        bail!("End block root mismatch at {}: db={}, rpc={}", end_block, end_root, rpc_end_root);
    }

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
        read_stats: recorder.stats(),
        rpc_read_fallback_count: rpc_stats.read_fallback_count.load(Ordering::Relaxed),
        rpc_read_fallback_total_ms: rpc_stats.read_fallback_total_ms.load(Ordering::Relaxed),
        rpc_state_diff_count: rpc_stats.state_diff_count.load(Ordering::Relaxed),
        rpc_state_diff_total_ms: rpc_stats.state_diff_total_ms.load(Ordering::Relaxed),
        calls_path: calls_path.display().to_string(),
        roots_path: roots_path.display().to_string(),
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
    let mut storage_diffs: Vec<ContractStorageDiffItem> = Vec::new();
    for contract_diff in raw_state_diff.storage_diffs {
        let ContractStorageDiffItem { address, storage_entries } = contract_diff;
        let filtered_entries: Vec<StorageEntry> =
            storage_entries.into_iter().filter(|entry| entry.value != Felt::ZERO).collect();
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
        declared_classes: raw_state_diff.declared_classes,
        old_declared_contracts: raw_state_diff.old_declared_contracts,
        nonces: raw_state_diff.nonces,
        replaced_classes,
        migrated_compiled_classes: raw_state_diff.migrated_compiled_classes,
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
