use anyhow::{anyhow, Context, Result};
use clap::{Parser, ValueEnum};
use mc_class_exec::config::NativeConfig;
use mc_class_exec::init_compilation_semaphore;
use mc_db::{
    rocksdb::{global_trie::bonsai_identifier, trie::WrappedBonsaiError, RocksDBConfig},
    MadaraBackend, MadaraBackendConfig, MadaraStorageRead,
};
use mp_chain_config::ChainConfig;
use mp_state_update::{
    ContractStorageDiffItem, DeclaredClassItem, DeployedContractItem, MigratedClassItem, NonceUpdate, ReplacedClassItem,
    StateDiff, StorageEntry,
};
use serde::{Deserialize, Serialize};
use starknet_types_core::felt::Felt;
use starknet_types_core::hash::{Pedersen, StarkHash};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use futures::{stream, StreamExt, TryStreamExt};

#[derive(Clone, Debug, Serialize, Deserialize, ValueEnum)]
enum Mode {
    Sequential,
    Parallel,
    Both,
}

#[derive(Clone, Debug, Serialize, Deserialize, ValueEnum)]
enum CopyStrategy {
    Full,
}

#[derive(Clone, Debug, Serialize, Deserialize, ValueEnum)]
enum ParallelApply {
    Batched,
    Squashed,
}

#[derive(Parser, Debug)]
#[command(author, version, about = "Madara global trie POC runner")]
struct Cli {
    /// Base Madara DB path (directory containing the `db/` folder)
    #[arg(long)]
    db_path: PathBuf,

    /// Output JSON path
    #[arg(long)]
    output: Option<PathBuf>,

    /// Work directory for DB copies
    #[arg(long, default_value = "./trie-poc-work")]
    work_dir: PathBuf,

    /// Start block (base state). If omitted, uses fixture start-1 or latest_confirmed - block_count.
    #[arg(long)]
    start_block: Option<u64>,

    /// Block count to process
    #[arg(long, default_value_t = 10)]
    block_count: u64,

    /// Number of DB copies for parallel
    #[arg(long, default_value_t = 5)]
    copies: usize,

    /// Mode to run
    #[arg(long, value_enum, default_value_t = Mode::Both)]
    mode: Mode,

    /// Parallel apply mode
    #[arg(long, value_enum, default_value_t = ParallelApply::Batched)]
    parallel_apply: ParallelApply,

    /// Enable pre-range checks when squashing state diffs
    #[arg(long, default_value_t = false)]
    squash_pre_range: bool,

    /// Dump contract leaf debug info for a specific block
    #[arg(long)]
    debug_block: Option<u64>,

    /// Output directory for debug dumps (required when --debug-block is set)
    #[arg(long)]
    debug_output_dir: Option<PathBuf>,

    /// Chain ID for opening the DB (e.g. MADARA_DEVNET or POC_DEVNET)
    #[arg(long, default_value = "MADARA_DEVNET")]
    chain_id: String,

    /// Copy strategy (POC defaults to full DB copy)
    #[arg(long, value_enum, default_value_t = CopyStrategy::Full)]
    copy_strategy: CopyStrategy,

    /// Read state diffs from JSON (fixture format). If omitted, read from DB.
    #[arg(long)]
    state_diffs_json: Option<PathBuf>,

    /// Allow running even if the base DB is ahead of start_block
    #[arg(long, default_value_t = false)]
    allow_non_base: bool,

    /// Overwrite existing work dir
    #[arg(long, default_value_t = false)]
    overwrite_work_dir: bool,
}

#[derive(Debug, Deserialize)]
struct Fixture {
    block_range: [u64; 2],
    blocks: Vec<FixtureBlock>,
}

#[derive(Debug, Deserialize)]
struct FixtureBlock {
    block_number: u64,
    block_hash: Felt,
    old_root: Felt,
    new_root: Felt,
    state_diff: RpcStateDiff,
}

#[derive(Debug, Deserialize)]
struct RpcStateDiff {
    declared_classes: Vec<DeclaredClassItem>,
    deployed_contracts: Vec<DeployedContractItem>,
    #[serde(default, rename = "deprecated_declared_classes")]
    old_declared_contracts: Vec<Felt>,
    nonces: Vec<NonceUpdate>,
    replaced_classes: Vec<ReplacedClassItem>,
    storage_diffs: Vec<ContractStorageDiffItem>,
    #[serde(default)]
    migrated_compiled_classes: Vec<MigratedClassItem>,
}

impl From<RpcStateDiff> for StateDiff {
    fn from(value: RpcStateDiff) -> Self {
        Self {
            storage_diffs: value.storage_diffs,
            old_declared_contracts: value.old_declared_contracts,
            declared_classes: value.declared_classes,
            deployed_contracts: value.deployed_contracts,
            replaced_classes: value.replaced_classes,
            nonces: value.nonces,
            migrated_compiled_classes: value.migrated_compiled_classes,
        }
    }
}

#[derive(Debug, Serialize)]
struct Output {
    run_id: String,
    timestamp_unix_ms: u128,
    db_path: PathBuf,
    work_dir: PathBuf,
    start_block: u64,
    block_count: u64,
    copies: usize,
    mode: Mode,
    copy_strategy: CopyStrategy,
    state_diff_source: String,
    diff_load_ms: u128,
    output_write_ms: u128,
    parallel_apply: ParallelApply,
    diffs: Vec<OutputDiffBlock>,
    correctness: Option<Correctness>,
    sequential: Option<RunResult>,
    parallel: Option<RunResult>,
}

#[derive(Debug, Serialize)]
struct OutputDiffBlock {
    block_number: u64,
    block_hash: Option<Felt>,
    old_root: Option<Felt>,
    new_root: Option<Felt>,
    state_diff: StateDiff,
}

#[derive(Debug, Serialize)]
struct RunResult {
    total_ms: u128,
    per_block: Vec<BlockResult>,
    copy_times_ms: Vec<CopyTime>,
}

#[derive(Debug, Serialize)]
struct CopyTime {
    copy_id: String,
    ms: u128,
}

#[derive(Clone, Debug, Serialize)]
struct BlockResult {
    block_number: u64,
    root: Felt,
    contract_root: Felt,
    class_root: Felt,
    elapsed_ms: u128,
    elapsed_us: u128,
    copy_index: Option<usize>,
    squash_ms: Option<u128>,
    squash_us: Option<u128>,
}

#[derive(Debug, Serialize)]
struct Correctness {
    matched: bool,
    mismatches: Vec<RootMismatch>,
    contract_trie_mismatches: Vec<RootMismatch>,
    class_trie_mismatches: Vec<RootMismatch>,
}

#[derive(Debug, Serialize)]
struct RootMismatch {
    block_number: u64,
    sequential: Felt,
    parallel: Felt,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    ensure_work_dir(&cli.work_dir, cli.overwrite_work_dir)?;

    let load_start = Instant::now();
    let (state_diffs, diff_meta, fixture_start_block, source) = load_state_diffs(&cli)?;
    let diff_load_ms = load_start.elapsed().as_millis();
    let start_block = cli
        .start_block
        .or(fixture_start_block)
        .unwrap_or_else(|| infer_start_block_from_db(&cli).unwrap_or(0));

    let base_backend = open_backend(&cli.db_path, &cli.chain_id)?;
    let latest_confirmed = base_backend.latest_confirmed_block_n().unwrap_or(0);
    if latest_confirmed > start_block && !cli.allow_non_base {
        return Err(anyhow!(
            "base DB confirmed tip {} is ahead of start_block {}. Use a base DB at start_block or pass --allow-non-base",
            latest_confirmed,
            start_block
        ));
    }

    let diffs = state_diffs;
    let output_diffs = diff_meta;

    let mut sequential = None;
    let mut parallel = None;

    if matches!(cli.mode, Mode::Sequential | Mode::Both) {
        let seq_dir = cli.work_dir.join("seq");
        let seq_copy = make_copy(&cli.db_path, &seq_dir, cli.copy_strategy.clone())?;
    let seq_backend = open_backend(&seq_copy.path, &cli.chain_id)?;
        let (per_block, total) =
            run_sequential(seq_backend, start_block, &diffs, cli.debug_block, cli.debug_output_dir.as_deref())?;
        sequential = Some(RunResult {
            total_ms: total.as_millis(),
            per_block,
            copy_times_ms: vec![CopyTime { copy_id: "seq".into(), ms: seq_copy.copy_ms }],
        });
    }

    if matches!(cli.mode, Mode::Parallel | Mode::Both) {
        let mut copy_infos = Vec::with_capacity(cli.copies);
        let mut copy_paths = Vec::with_capacity(cli.copies);
        for i in 0..cli.copies {
            let copy_dir = cli.work_dir.join(format!("copy_{i}"));
            let info = make_copy(&cli.db_path, &copy_dir, cli.copy_strategy.clone())?;
            copy_paths.push(info.path.clone());
            copy_infos.push(info);
        }

        let (per_block, total) = run_parallel(
            &copy_paths,
            start_block,
            cli.copies,
            &diffs,
            &cli.chain_id,
            &cli.parallel_apply,
            cli.squash_pre_range,
            cli.debug_block,
            cli.debug_output_dir.as_deref(),
        )?;
        parallel = Some(RunResult {
            total_ms: total.as_millis(),
            per_block,
            copy_times_ms: copy_infos
                .into_iter()
                .map(|c| CopyTime { copy_id: c.name, ms: c.copy_ms })
                .collect(),
        });
    }

    let run_id = format!("madara-trie-poc-{}", SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs());
    let correctness = compare_roots(sequential.as_ref(), parallel.as_ref());
    let output = Output {
        run_id,
        timestamp_unix_ms: SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis(),
        db_path: cli.db_path,
        work_dir: cli.work_dir,
        start_block,
        block_count: cli.block_count,
        copies: cli.copies,
        mode: cli.mode,
        copy_strategy: cli.copy_strategy,
        state_diff_source: source,
        diff_load_ms,
        output_write_ms: 0,
        parallel_apply: cli.parallel_apply,
        diffs: output_diffs,
        correctness,
        sequential,
        parallel,
    };

    let out_path = cli.output.unwrap_or_else(|| PathBuf::from("trie-poc-results.json"));
    let serialized = serde_json::to_string_pretty(&output).context("serializing output json")?;
    let output_write_start = Instant::now();
    fs::write(&out_path, serialized).context("writing output json")?;
    let output_write_ms = output_write_start.elapsed().as_millis();
    if output_write_ms > 0 {
        let mut output = output;
        output.output_write_ms = output_write_ms;
        let serialized = serde_json::to_string_pretty(&output).context("serializing output json (final)")?;
        fs::write(&out_path, serialized).context("writing output json (final)")?;
    }
    println!("wrote results to {}", out_path.display());
    Ok(())
}

struct CopyInfo {
    name: String,
    path: PathBuf,
    copy_ms: u128,
}

fn ensure_work_dir(dir: &Path, overwrite: bool) -> Result<()> {
    if dir.exists() {
        if !overwrite {
            return Err(anyhow!(
                "work dir {} already exists; pass --overwrite-work-dir to replace",
                dir.display()
            ));
        }
        fs::remove_dir_all(dir).context("removing existing work dir")?;
    }
    fs::create_dir_all(dir).context("creating work dir")?;
    Ok(())
}

fn make_copy(base: &Path, dest: &Path, _strategy: CopyStrategy) -> Result<CopyInfo> {
    if dest.exists() {
        fs::remove_dir_all(dest).context("removing existing copy dir")?;
    }
    fs::create_dir_all(dest).context("creating copy dir")?;
    let start = Instant::now();
    copy_dir_recursive(base, dest)?;
    Ok(CopyInfo {
        name: dest
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "copy".into()),
        path: dest.to_path_buf(),
        copy_ms: start.elapsed().as_millis(),
    })
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    for entry in fs::read_dir(src).context("read_dir failed")? {
        let entry = entry.context("read_dir entry failed")?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        let file_type = entry.file_type().context("file_type failed")?;
        if file_type.is_dir() {
            fs::create_dir_all(&dst_path).context("create_dir_all failed")?;
            copy_dir_recursive(&src_path, &dst_path)?;
        } else if file_type.is_file() {
            fs::copy(&src_path, &dst_path).with_context(|| format!("copy failed for {}", src_path.display()))?;
        }
    }
    Ok(())
}

fn open_backend(db_path: &Path, chain_id: &str) -> Result<Arc<MadaraBackend>> {
    let mut config = ChainConfig::madara_devnet();
    config.chain_id = starknet_api::core::ChainId::Other(chain_id.into());
    let chain_config = Arc::new(config);
    let rocksdb_config = RocksDBConfig::default();
    let backend_config = MadaraBackendConfig::default();

    let native_builder = NativeConfig::builder();
    let max_concurrent = native_builder.max_concurrent_compilations();
    init_compilation_semaphore(max_concurrent);
    let cairo_native_config = Arc::new(native_builder.build());

    MadaraBackend::open_rocksdb(db_path, chain_config, backend_config, rocksdb_config, cairo_native_config)
        .context("opening madara rocksdb")
}

fn infer_start_block_from_db(cli: &Cli) -> Result<u64> {
    let backend = open_backend(&cli.db_path, &cli.chain_id)?;
    let latest = backend.latest_confirmed_block_n().unwrap_or(0);
    Ok(latest.saturating_sub(cli.block_count))
}

fn load_state_diffs(cli: &Cli) -> Result<(Vec<StateDiff>, Vec<OutputDiffBlock>, Option<u64>, String)> {
    if let Some(path) = &cli.state_diffs_json {
        let raw = fs::read_to_string(path).with_context(|| format!("reading fixture {}", path.display()))?;
        let mut fixture: Fixture = serde_json::from_str(&raw).context("parsing fixture json")?;
        fixture.blocks.sort_by_key(|b| b.block_number);
        let start_block = fixture.block_range[0].saturating_sub(1);

        let mut diffs = Vec::with_capacity(fixture.blocks.len());
        let mut output = Vec::with_capacity(fixture.blocks.len());
        for block in fixture.blocks {
            let state_diff: StateDiff = block.state_diff.into();
            diffs.push(state_diff.clone());
            output.push(OutputDiffBlock {
                block_number: block.block_number,
                block_hash: Some(block.block_hash),
                old_root: Some(block.old_root),
                new_root: Some(block.new_root),
                state_diff,
            });
        }
        return Ok((diffs, output, Some(start_block), "json".into()));
    }

    let backend = open_backend(&cli.db_path, &cli.chain_id)?;
    let start_block = cli.start_block.unwrap_or_else(|| backend.latest_confirmed_block_n().unwrap_or(0).saturating_sub(cli.block_count));
    let mut diffs = Vec::with_capacity(cli.block_count as usize);
    let mut output = Vec::with_capacity(cli.block_count as usize);

    for block_number in (start_block + 1)..=(start_block + cli.block_count) {
        let state_diff = backend
            .db
            .get_block_state_diff(block_number)
            .with_context(|| format!("get_block_state_diff for block {}", block_number))?
            .ok_or_else(|| anyhow!("missing state diff for block {}", block_number))?;
        let block_info = backend.db.get_block_info(block_number)?;
        output.push(OutputDiffBlock {
            block_number,
            block_hash: block_info.as_ref().map(|info| info.block_hash),
            old_root: None,
            new_root: block_info.as_ref().map(|info| info.header.global_state_root),
            state_diff: state_diff.clone(),
        });
        diffs.push(state_diff);
    }
    Ok((diffs, output, Some(start_block), "db".into()))
}

fn run_sequential(
    backend: Arc<MadaraBackend>,
    start_block: u64,
    diffs: &[StateDiff],
    debug_block: Option<u64>,
    debug_output_dir: Option<&Path>,
) -> Result<(Vec<BlockResult>, Duration)> {
    let mut results = Vec::with_capacity(diffs.len());
    let start = Instant::now();
    let pool = rayon::ThreadPoolBuilder::new().build().context("building rayon pool")?;
    let backend = Arc::clone(&backend);
    pool.install(|| {
        for (i, diff) in diffs.iter().enumerate() {
            let block_number = start_block + i as u64 + 1;
            let mut diff = diff.clone();
            diff.sort();
            let t = Instant::now();
            backend
                .write_access()
                .write_state_diff(block_number, &diff)
                .with_context(|| format!("write_state_diff block {}", block_number))?;
            let root = backend
                .write_access()
                .apply_to_global_trie(block_number, [&diff])
                .with_context(|| format!("apply_to_global_trie block {}", block_number))?;
            let (contract_root, class_root) = read_trie_roots(&backend.db)?;
            if debug_block == Some(block_number) {
                dump_contract_leaf_debug(
                    &backend,
                    block_number,
                    &diff,
                    debug_output_dir,
                    "seq",
                )?;
            }
            results.push(BlockResult {
                block_number,
                root,
                contract_root,
                class_root,
                elapsed_ms: t.elapsed().as_millis(),
                elapsed_us: t.elapsed().as_micros(),
                copy_index: None,
                squash_ms: None,
                squash_us: None,
            });
        }
        Ok::<(), anyhow::Error>(())
    })?;
    Ok((results, start.elapsed()))
}

fn run_parallel(
    copy_paths: &[PathBuf],
    start_block: u64,
    copies: usize,
    diffs: &[StateDiff],
    chain_id: &str,
    parallel_apply: &ParallelApply,
    squash_pre_range: bool,
    debug_block: Option<u64>,
    debug_output_dir: Option<&Path>,
) -> Result<(Vec<BlockResult>, Duration)> {
    if copies == 0 {
        return Err(anyhow!("copies must be > 0"));
    }
    let block_count = diffs.len();
    let mut assignments: Vec<Vec<usize>> = vec![Vec::new(); copies];
    for idx in 0..block_count {
        assignments[idx % copies].push(idx);
    }

    let start = Instant::now();
    let runtime = Arc::new(
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .context("building tokio runtime")?,
    );
    let results = Arc::new(std::sync::Mutex::new(Vec::with_capacity(block_count)));
    let diffs = Arc::new(diffs.to_vec());

    rayon::scope(|scope| {
        for (copy_idx, copy_path) in copy_paths.iter().enumerate() {
            let diffs = Arc::clone(&diffs);
            let assigned = assignments[copy_idx].clone();
            let results = Arc::clone(&results);
            let chain_id = chain_id.to_string();
            let parallel_apply = parallel_apply.clone();
            let runtime = Arc::clone(&runtime);
            let debug_block = debug_block;
            let debug_output_dir = debug_output_dir.map(Path::to_path_buf);
            scope.spawn(move |_| {
                if assigned.is_empty() {
                    return;
                }
                let backend = match open_backend(copy_path, &chain_id) {
                    Ok(b) => b,
                    Err(err) => {
                        eprintln!("failed to open copy {}: {err:#}", copy_path.display());
                        return;
                    }
                };
                let mut current_index: isize = -1;
                for target_index in assigned {
                    let block_number = start_block + target_index as u64 + 1;
                    let from = (current_index + 1) as usize;
                    let to = target_index;
                    let apply_start = Instant::now();
                    let (root, squash_us) = match parallel_apply {
                        ParallelApply::Batched => {
                            let mut batch: Vec<StateDiff> = diffs[from..=to].iter().cloned().collect();
                            for diff in &mut batch {
                                diff.sort();
                            }
                            for (idx, diff) in batch.iter().enumerate() {
                                let block_n = start_block + from as u64 + 1 + idx as u64;
                                if let Err(err) = backend.write_access().write_state_diff(block_n, diff) {
                                    eprintln!("write_state_diff failed for block {}: {err:#}", block_n);
                                    return;
                                }
                            }
                            let root = backend
                                .write_access()
                                .apply_to_global_trie(start_block + from as u64 + 1, batch.iter());
                            (root, None)
                        }
                        ParallelApply::Squashed => {
                            // Ensure state columns are updated up to `to`
                            for idx in (current_index + 1) as usize..=to {
                                let block_n = start_block + idx as u64 + 1;
                                if let Err(err) = backend.write_access().write_state_diff(block_n, &diffs[idx]) {
                                    eprintln!("write_state_diff failed for block {}: {err:#}", block_n);
                                    return;
                                }
                            }

                            let mut map = StateDiffMap::default();
                            for diff in &diffs[from..=to] {
                                map.apply_state_diff(diff);
                            }
                            let mut raw = map.to_raw_state_diff();
                            raw.sort();
                            let pre_range_block = if squash_pre_range {
                                Some(start_block + from as u64)
                            } else {
                                None
                            };

                            let squash_start = Instant::now();
                            let compressed = match runtime.block_on(compress_state_diff(raw, pre_range_block, backend.clone())) {
                                Ok(c) => c,
                                Err(err) => {
                                    eprintln!("compress_state_diff failed for block {}: {err:#}", block_number);
                                    return;
                                }
                            };
                            let squash_us = Some(squash_start.elapsed().as_micros());
                            let root = backend.write_access().apply_to_global_trie(block_number, [&compressed]);
                            (root, squash_us)
                        }
                    };
                    let root = match root {
                        Ok(r) => r,
                        Err(err) => {
                            eprintln!("apply_to_global_trie failed for block {}: {err:#}", block_number);
                            return;
                        }
                    };
                    let (contract_root, class_root) = match read_trie_roots(&backend.db) {
                        Ok(r) => r,
                        Err(err) => {
                            eprintln!("read_trie_roots failed for block {}: {err:#}", block_number);
                            return;
                        }
                    };
                    if debug_block == Some(block_number) {
                        let diff_for_debug = match parallel_apply {
                            ParallelApply::Batched => {
                                let mut batch: Vec<StateDiff> = diffs[from..=to].iter().cloned().collect();
                                for diff in &mut batch {
                                    diff.sort();
                                }
                                let mut map = StateDiffMap::default();
                                for diff in &batch {
                                    map.apply_state_diff(diff);
                                }
                                map.to_raw_state_diff()
                            }
                            ParallelApply::Squashed => {
                                let mut map = StateDiffMap::default();
                                for diff in &diffs[from..=to] {
                                    map.apply_state_diff(diff);
                                }
                                map.to_raw_state_diff()
                            }
                        };
                        if let Err(err) = dump_contract_leaf_debug(
                            &backend,
                            block_number,
                            &diff_for_debug,
                            debug_output_dir.as_deref(),
                            &format!("copy_{copy_idx}"),
                        ) {
                            eprintln!("dump_contract_leaf_debug failed for block {}: {err:#}", block_number);
                            return;
                        }
                    }
                    let elapsed_ms = apply_start.elapsed().as_millis();
                    let elapsed_us = apply_start.elapsed().as_micros();
                    if let Ok(mut guard) = results.lock() {
                        guard.push(BlockResult {
                            block_number,
                            root,
                            contract_root,
                            class_root,
                            elapsed_ms,
                            elapsed_us,
                            copy_index: Some(copy_idx),
                            squash_ms: squash_us.map(|us| us / 1000),
                            squash_us,
                        });
                    }
                    current_index = target_index as isize;
                }
            });
        }
    });

    let mut all_results = results.lock().map_err(|_| anyhow!("results mutex poisoned"))?.clone();
    all_results.sort_by_key(|r| r.block_number);
    Ok((all_results, start.elapsed()))
}


fn compare_roots(sequential: Option<&RunResult>, parallel: Option<&RunResult>) -> Option<Correctness> {
    let seq = sequential?;
    let par = parallel?;
    let mut seq_map = HashMap::new();
    let mut seq_contract = HashMap::new();
    let mut seq_class = HashMap::new();
    for entry in &seq.per_block {
        seq_map.insert(entry.block_number, entry.root);
        seq_contract.insert(entry.block_number, entry.contract_root);
        seq_class.insert(entry.block_number, entry.class_root);
    }

    let mut mismatches = Vec::new();
    let mut contract_trie_mismatches = Vec::new();
    let mut class_trie_mismatches = Vec::new();
    for entry in &par.per_block {
        if let Some(seq_root) = seq_map.get(&entry.block_number) {
            if seq_root != &entry.root {
                mismatches.push(RootMismatch {
                    block_number: entry.block_number,
                    sequential: *seq_root,
                    parallel: entry.root,
                });
            }
        }
        if let Some(seq_root) = seq_contract.get(&entry.block_number) {
            if seq_root != &entry.contract_root {
                contract_trie_mismatches.push(RootMismatch {
                    block_number: entry.block_number,
                    sequential: *seq_root,
                    parallel: entry.contract_root,
                });
            }
        }
        if let Some(seq_root) = seq_class.get(&entry.block_number) {
            if seq_root != &entry.class_root {
                class_trie_mismatches.push(RootMismatch {
                    block_number: entry.block_number,
                    sequential: *seq_root,
                    parallel: entry.class_root,
                });
            }
        }
    }

    Some(Correctness {
        matched: mismatches.is_empty() && contract_trie_mismatches.is_empty() && class_trie_mismatches.is_empty(),
        mismatches,
        contract_trie_mismatches,
        class_trie_mismatches,
    })
}

fn read_trie_roots(db: &mc_db::rocksdb::RocksDBStorage) -> Result<(Felt, Felt)> {
    let contract_trie = db.contract_trie();
    let contract_root = contract_trie
        .root_hash(bonsai_identifier::CONTRACT)
        .map_err(WrappedBonsaiError)?;

    let class_trie = db.class_trie();
    let class_root = class_trie.root_hash(bonsai_identifier::CLASS).map_err(WrappedBonsaiError)?;

    Ok((contract_root, class_root))
}

#[derive(Serialize)]
struct ContractLeafDebug {
    address: Felt,
    class_hash: Felt,
    nonce: Felt,
    storage_root: Felt,
    leaf_hash: Felt,
}

#[derive(Serialize)]
struct ContractLeafDebugDump {
    block_number: u64,
    label: String,
    contracts: Vec<ContractLeafDebug>,
}

fn dump_contract_leaf_debug(
    backend: &Arc<MadaraBackend>,
    block_number: u64,
    diff: &StateDiff,
    output_dir: Option<&Path>,
    label: &str,
) -> Result<()> {
    let output_dir = output_dir.ok_or_else(|| anyhow!("--debug-output-dir required when --debug-block is set"))?;
    fs::create_dir_all(output_dir).context("create debug output dir")?;

    let mut addresses = HashSet::new();
    for item in &diff.storage_diffs {
        addresses.insert(item.address);
    }
    for item in &diff.deployed_contracts {
        addresses.insert(item.address);
    }
    for item in &diff.replaced_classes {
        addresses.insert(item.contract_address);
    }
    for item in &diff.nonces {
        addresses.insert(item.contract_address);
    }

    let contract_storage_trie = backend.db.contract_storage_trie();

    let mut contracts = Vec::new();
    for address in addresses {
        let storage_root = contract_storage_trie
            .root_hash(&address.to_bytes_be())
            .map_err(WrappedBonsaiError)?;
        let nonce = backend
            .db
            .get_contract_nonce_at(block_number, &address)?
            .unwrap_or(Felt::ZERO);
        let class_hash = backend
            .db
            .get_contract_class_hash_at(block_number, &address)?
            .unwrap_or(Felt::ZERO);
        let leaf_hash = Pedersen::hash(
            &Pedersen::hash(&Pedersen::hash(&class_hash, &storage_root), &nonce),
            &Felt::ZERO,
        );
        contracts.push(ContractLeafDebug { address, class_hash, nonce, storage_root, leaf_hash });
    }
    contracts.sort_by_key(|c| c.address);

    let dump = ContractLeafDebugDump { block_number, label: label.to_string(), contracts };
    let out_path = output_dir.join(format!("{label}_block_{block_number}.json"));
    let serialized = serde_json::to_string_pretty(&dump).context("serializing debug dump")?;
    fs::write(&out_path, serialized).context("writing debug dump")?;
    Ok(())
}


const MAX_CONCURRENT_CONTRACTS_PROCESSING: usize = 400;
const MAX_CONCURRENT_GET_STORAGE_AT_CALLS: usize = 10000;

async fn compress_state_diff(
    raw_state_diff: StateDiff,
    pre_range_block: Option<u64>,
    backend: Arc<MadaraBackend>,
) -> Result<StateDiff> {
    let storage_diffs = stream::iter(raw_state_diff.storage_diffs.into_iter().enumerate())
        .map(|(_idx, contract_diff)| {
            let backend = backend.clone();
            async move {
                compress_single_contract(contract_diff.address, contract_diff.storage_entries, backend, pre_range_block)
                    .await
            }
        })
        .buffer_unordered(MAX_CONCURRENT_CONTRACTS_PROCESSING)
        .try_filter_map(|contract_storage_diff| async move { Ok(contract_storage_diff) })
        .try_collect::<Vec<_>>()
        .await?;

    let deployed_contracts_map: HashMap<Felt, Felt> =
        raw_state_diff.deployed_contracts.into_iter().map(|item| (item.address, item.class_hash)).collect();

    let replaced_classes_map: HashMap<Felt, Felt> = raw_state_diff
        .replaced_classes
        .into_iter()
        .map(|item| (item.contract_address, item.class_hash))
        .collect();

    let (replaced_classes, deployed_contracts) = process_deployed_contracts_and_replaced_classes(
        backend.clone(),
        pre_range_block,
        deployed_contracts_map,
        replaced_classes_map,
    )
    .await?;

    let mut compressed_diff = StateDiff {
        storage_diffs,
        deployed_contracts,
        declared_classes: raw_state_diff.declared_classes,
        old_declared_contracts: raw_state_diff.old_declared_contracts,
        nonces: raw_state_diff.nonces,
        replaced_classes,
        migrated_compiled_classes: raw_state_diff.migrated_compiled_classes,
    };

    compressed_diff.sort();

    Ok(compressed_diff)
}

async fn compress_single_contract(
    contract_addr: Felt,
    storage_entries: Vec<StorageEntry>,
    backend: Arc<MadaraBackend>,
    pre_range_block_option: Option<u64>,
) -> Result<Option<ContractStorageDiffItem>> {
    let storage_entries = match pre_range_block_option {
        Some(pre_range_block) => {
            let contract_existed = check_contract_existed_at_block(backend.clone(), contract_addr, pre_range_block).await?;
            let compressed_entries = if contract_existed {
                stream::iter(storage_entries)
                    .map(|entry| {
                        let backend = backend.clone();
                        async move {
                            let pre_range_value =
                                check_pre_range_storage_value(backend, contract_addr, entry.key, pre_range_block).await;
                            (entry, pre_range_value)
                        }
                    })
                    .buffer_unordered(MAX_CONCURRENT_GET_STORAGE_AT_CALLS)
                    .filter_map(|(entry, pre_range_value)| async move {
                        if pre_range_value != Some(entry.value) {
                            Some(entry)
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
                    .await
            } else {
                storage_entries.into_iter().filter(|entry| entry.value != Felt::ZERO).collect()
            };
            compressed_entries
        }
        None => storage_entries.into_iter().filter(|entry| entry.value != Felt::ZERO).collect(),
    };

    if !storage_entries.is_empty() {
        Ok(Some(ContractStorageDiffItem { address: contract_addr, storage_entries }))
    } else {
        Ok(None)
    }
}

async fn process_deployed_contracts_and_replaced_classes(
    backend: Arc<MadaraBackend>,
    pre_range_block_option: Option<u64>,
    mut deployed_contracts: HashMap<Felt, Felt>,
    mut replaced_class_hashes: HashMap<Felt, Felt>,
) -> Result<(Vec<ReplacedClassItem>, Vec<DeployedContractItem>)> {
    replaced_class_hashes.retain(|contract_address, class_hash| {
        if let Some(existing) = deployed_contracts.get(contract_address).copied() {
            deployed_contracts.insert(*contract_address, *class_hash);
            existing != *class_hash
        } else {
            true
        }
    });

    let replaced_classes = match pre_range_block_option {
        Some(pre_range_block) => {
            let backend = backend.clone();
            stream::iter(replaced_class_hashes)
                .map(|(contract_address, class_hash)| {
                    let backend = backend.clone();
                    async move {
                        let prev_hash =
                            check_pre_range_class_hash(backend, contract_address, pre_range_block).await;
                        if prev_hash != Some(class_hash) {
                            Some(ReplacedClassItem { contract_address, class_hash })
                        } else {
                            None
                        }
                    }
                })
                .buffer_unordered(MAX_CONCURRENT_CONTRACTS_PROCESSING)
                .filter_map(|item| async move { item })
                .collect::<Vec<_>>()
                .await
        }
        None => replaced_class_hashes
            .into_iter()
            .map(|(contract_address, class_hash)| ReplacedClassItem { contract_address, class_hash })
            .collect(),
    };

    let deployed_contracts = deployed_contracts
        .into_iter()
        .map(|(address, class_hash)| DeployedContractItem { address, class_hash })
        .collect();

    Ok((replaced_classes, deployed_contracts))
}

async fn check_contract_existed_at_block(
    backend: Arc<MadaraBackend>,
    contract_address: Felt,
    block_n: u64,
) -> Result<bool> {
    backend
        .db
        .is_contract_deployed_at(block_n, &contract_address)
        .context("is_contract_deployed_at")
}

async fn check_pre_range_storage_value(
    backend: Arc<MadaraBackend>,
    contract_address: Felt,
    key: Felt,
    pre_range_block: u64,
) -> Option<Felt> {
    backend
        .db
        .get_storage_at(pre_range_block, &contract_address, &key)
        .ok()
        .flatten()
}

async fn check_pre_range_class_hash(
    backend: Arc<MadaraBackend>,
    contract_address: Felt,
    pre_range_block: u64,
) -> Option<Felt> {
    backend
        .db
        .get_contract_class_hash_at(pre_range_block, &contract_address)
        .ok()
        .flatten()
}

#[derive(Default)]
struct StateDiffMap {
    storage_diffs: HashMap<Felt, HashMap<Felt, Felt>>,
    deployed_contracts: HashMap<Felt, Felt>,
    declared_classes: HashMap<Felt, Felt>,
    deprecated_declared_classes: HashSet<Felt>,
    nonces: HashMap<Felt, Felt>,
    replaced_classes: HashMap<Felt, Felt>,
    touched_contracts: HashSet<Felt>,
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
            .clone()
            .into_iter()
            .filter_map(|contract_addr| {
                self.storage_diffs.get(&contract_addr).map(|storage_map| {
                    let storage_entries: Vec<StorageEntry> =
                        storage_map.iter().map(|(key, value)| StorageEntry { key: *key, value: *value }).collect();
                    ContractStorageDiffItem { address: contract_addr, storage_entries }
                })
            })
            .filter(|item| !item.storage_entries.is_empty())
            .collect();

        let deployed_contracts = self
            .deployed_contracts
            .clone()
            .into_iter()
            .map(|(address, class_hash)| DeployedContractItem { address, class_hash })
            .collect();

        let declared_classes = self
            .declared_classes
            .clone()
            .into_iter()
            .map(|(class_hash, compiled_class_hash)| DeclaredClassItem { class_hash, compiled_class_hash })
            .collect();

        let nonces = self
            .nonces
            .clone()
            .into_iter()
            .map(|(contract_address, nonce)| NonceUpdate { contract_address, nonce })
            .collect();

        let replaced_classes = self
            .replaced_classes
            .clone()
            .into_iter()
            .map(|(contract_address, class_hash)| ReplacedClassItem { contract_address, class_hash })
            .collect();

        let deprecated_declared_classes = self.deprecated_declared_classes.clone().into_iter().collect();

        let migrated_compiled_classes = self
            .migrated_compiled_classes
            .clone()
            .into_iter()
            .map(|(class_hash, compiled_class_hash)| MigratedClassItem { class_hash, compiled_class_hash })
            .collect();

        StateDiff {
            storage_diffs,
            deployed_contracts,
            declared_classes,
            old_declared_contracts: deprecated_declared_classes,
            nonces,
            replaced_classes,
            migrated_compiled_classes,
        }
    }
}
