use anyhow::{bail, Context};
use blockifier::blockifier_versioned_constants::VersionedConstants;
use flate2::{read::GzDecoder, write::GzEncoder, Compression};
use generate_pie::types::{ChainConfig, OsHintsConfiguration, PieGenerationInput};
use mc_db::MadaraBackend;
use mc_telemetry::{
    register_counter_metric_instrument, register_gauge_metric_instrument, register_histogram_metric_instrument,
};
use opentelemetry::{
    global,
    metrics::{Counter, Gauge, Histogram},
    InstrumentationScope, KeyValue,
};
use serde::{Deserialize, Serialize};
use starknet_types_core::felt::Felt;
use std::collections::{BTreeSet, HashMap};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const RETENTION_ENV: &str = "MADARA_SNOS_WITNESS_RETENTION_BLOCKS";
const DIRECTORY_ENV: &str = "MADARA_SNOS_WITNESS_DIR";
const RPC_URL_ENV: &str = "MADARA_SNOS_WITNESS_RPC_URL";
const BUILD_CONCURRENCY_ENV: &str = "MADARA_SNOS_WITNESS_BUILD_CONCURRENCY";
const INITIAL_BACKFILL_ENV: &str = "MADARA_SNOS_WITNESS_INITIAL_BACKFILL_BLOCKS";
const VERSIONED_CONSTANTS_PATH_ENV: &str = "MADARA_SNOS_WITNESS_VERSIONED_CONSTANTS_PATH";
const DEFAULT_DIRECTORY: &str = "/data/snos-witnesses";
const DEFAULT_RPC_URL: &str = "http://127.0.0.1:9944";
const GENERATION_FLOOR_FILE: &str = ".generation-floor";
const SCHEDULER_INTERVAL: Duration = Duration::from_secs(1);
const BUILD_RETRY_DELAY: Duration = Duration::from_secs(30);

#[derive(Debug, Clone)]
struct SnosWitnessConfig {
    retention_blocks: u64,
    directory: PathBuf,
    rpc_url: String,
    build_concurrency: usize,
    initial_backfill_blocks: u64,
    versioned_constants: Option<VersionedConstants>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SnosWitnessManifest {
    pub schema_version: u32,
    pub block_number: u64,
    pub block_hash: Felt,
    pub state_root: Felt,
    pub response_count: usize,
    pub compressed_bytes: u64,
    pub uncompressed_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct SnosWitnessRangeStatus {
    pub ready: bool,
    pub start_block: u64,
    pub end_block: u64,
    pub witnessable_head: Option<u64>,
    pub oldest_retained_block: Option<u64>,
    pub generation_floor: u64,
    pub first_missing_block: Option<u64>,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum SnosWitnessAvailabilityError {
    #[error("SNOS witness for block {block_number} is pending")]
    Pending { block_number: u64 },
    #[error("block {block_number} is outside the retained SNOS witness range {oldest}..={head}")]
    OutsideRetention { block_number: u64, oldest: u64, head: u64 },
    #[error("block {block_number} predates the configured witness generation floor {generation_floor}")]
    BeforeGenerationFloor { block_number: u64, generation_floor: u64 },
    #[error("SNOS witness service has no trie-applied confirmed head yet")]
    NoWitnessableHead,
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

#[derive(Debug)]
pub(crate) struct SnosWitnessService {
    backend: Arc<MadaraBackend>,
    config: SnosWitnessConfig,
    generation_floor: Mutex<Option<u64>>,
    metrics: SnosWitnessMetrics,
}

#[derive(Debug)]
struct SnosWitnessMetrics {
    builds: Counter<u64>,
    build_duration: Histogram<f64>,
    active_builds: Gauge<u64>,
    witnessable_head: Gauge<u64>,
    generation_floor: Gauge<u64>,
    latest_ready_block: Gauge<u64>,
}

impl SnosWitnessMetrics {
    fn register() -> Self {
        let meter = global::meter_with_scope(
            InstrumentationScope::builder("crates.gateway.snos_witness.opentelemetry")
                .with_attributes([KeyValue::new("crate", "gateway")])
                .build(),
        );
        Self {
            builds: register_counter_metric_instrument(
                &meter,
                "snos_witness_build_total".to_owned(),
                "Completed SNOS witness build attempts".to_owned(),
                "build".to_owned(),
            ),
            build_duration: register_histogram_metric_instrument(
                &meter,
                "snos_witness_build_duration_seconds".to_owned(),
                "SNOS witness build duration".to_owned(),
                "s".to_owned(),
            ),
            active_builds: register_gauge_metric_instrument(
                &meter,
                "snos_witness_active_builds".to_owned(),
                "Currently active SNOS witness builds".to_owned(),
                "build".to_owned(),
            ),
            witnessable_head: register_gauge_metric_instrument(
                &meter,
                "snos_witness_witnessable_head".to_owned(),
                "Latest confirmed block with trie state applied".to_owned(),
                "block".to_owned(),
            ),
            generation_floor: register_gauge_metric_instrument(
                &meter,
                "snos_witness_generation_floor".to_owned(),
                "Durable first block eligible for witness generation".to_owned(),
                "block".to_owned(),
            ),
            latest_ready_block: register_gauge_metric_instrument(
                &meter,
                "snos_witness_latest_ready_block".to_owned(),
                "Latest block whose canonical witness was published".to_owned(),
                "block".to_owned(),
            ),
        }
    }

    fn record_scheduler(&self, witnessable_head: u64, generation_floor: u64, active_builds: usize) {
        self.witnessable_head.record(witnessable_head, &[]);
        self.generation_floor.record(generation_floor, &[]);
        self.active_builds.record(active_builds as u64, &[]);
    }

    fn record_build(&self, outcome: &'static str, duration: Duration, block_number: Option<u64>) {
        let attributes = [KeyValue::new("outcome", outcome)];
        self.builds.add(1, &attributes);
        self.build_duration.record(duration.as_secs_f64(), &attributes);
        if let Some(block_number) = block_number {
            self.latest_ready_block.record(block_number, &[]);
        }
    }
}

impl SnosWitnessService {
    pub(crate) fn from_env(backend: Arc<MadaraBackend>) -> anyhow::Result<Option<Arc<Self>>> {
        let Some(retention_blocks) = std::env::var(RETENTION_ENV).ok() else {
            return Ok(None);
        };
        let retention_blocks = retention_blocks.parse::<u64>().with_context(|| format!("Parsing {RETENTION_ENV}"))?;
        if retention_blocks == 0 {
            return Ok(None);
        }

        let directory: PathBuf = std::env::var(DIRECTORY_ENV).unwrap_or_else(|_| DEFAULT_DIRECTORY.to_owned()).into();
        std::fs::create_dir_all(&directory)
            .with_context(|| format!("Creating SNOS witness directory {}", directory.display()))?;
        let initial_backfill_blocks = std::env::var(INITIAL_BACKFILL_ENV)
            .unwrap_or_else(|_| "1".to_owned())
            .parse::<u64>()
            .with_context(|| format!("Parsing {INITIAL_BACKFILL_ENV}"))?;
        let generation_floor = load_generation_floor(&directory)?;

        let config = SnosWitnessConfig {
            retention_blocks,
            directory,
            rpc_url: std::env::var(RPC_URL_ENV).unwrap_or_else(|_| DEFAULT_RPC_URL.to_owned()),
            build_concurrency: std::env::var(BUILD_CONCURRENCY_ENV)
                .unwrap_or_else(|_| "2".to_owned())
                .parse::<usize>()
                .with_context(|| format!("Parsing {BUILD_CONCURRENCY_ENV}"))?,
            initial_backfill_blocks,
            versioned_constants: generate_pie::utils::load_versioned_constants(
                std::env::var(VERSIONED_CONSTANTS_PATH_ENV).ok().as_deref(),
            )
            .map_err(anyhow::Error::msg)?,
        };
        if config.build_concurrency == 0 {
            bail!("{BUILD_CONCURRENCY_ENV} must be greater than zero")
        }
        tracing::info!(
            retention_blocks,
            directory = %config.directory.display(),
            rpc_url = %config.rpc_url,
            build_concurrency = config.build_concurrency,
            generation_floor,
            "SNOS block witness generation enabled"
        );

        Ok(Some(Arc::new(Self {
            backend,
            config,
            generation_floor: Mutex::new(generation_floor),
            metrics: SnosWitnessMetrics::register(),
        })))
    }

    /// Runs a recoverable post-trie pipeline. Artifact presence is the durable queue:
    /// every missing block from the generation floor through the trie-applied confirmed
    /// head is retried until its manifest and compressed witness are both published.
    pub(crate) fn spawn_builder(self: Arc<Self>) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut builds = tokio::task::JoinSet::new();
            let mut active = BTreeSet::new();
            let mut retry_after = HashMap::<u64, Instant>::new();
            let mut scan_cursor = 0;

            loop {
                let Some(target) = self.witnessable_head() else {
                    tokio::time::sleep(SCHEDULER_INTERVAL).await;
                    continue;
                };
                let generation_floor = match self.generation_floor(target) {
                    Ok(generation_floor) => generation_floor,
                    Err(error) => {
                        tracing::error!(%error, "Failed to initialize SNOS witness generation floor");
                        tokio::time::sleep(BUILD_RETRY_DELAY).await;
                        continue;
                    }
                };
                let oldest = self.oldest_retained(target);
                let start = generation_floor.max(oldest);
                scan_cursor = scan_cursor.max(start);
                self.metrics.record_scheduler(target, generation_floor, builds.len());

                while builds.len() < self.config.build_concurrency {
                    let Some(block_number) =
                        self.next_missing_block(&mut scan_cursor, start, target, &active, &retry_after).await
                    else {
                        break;
                    };
                    active.insert(block_number);
                    let service = Arc::clone(&self);
                    builds.spawn(async move {
                        let started_at = Instant::now();
                        let result = service.build(block_number).await;
                        (block_number, started_at.elapsed(), result)
                    });
                }
                self.metrics.record_scheduler(target, generation_floor, builds.len());

                tokio::select! {
                    joined = builds.join_next(), if !builds.is_empty() => {
                        match joined {
                            Some(Ok((block_number, duration, Ok(())))) => {
                                active.remove(&block_number);
                                retry_after.remove(&block_number);
                                self.metrics.record_build("success", duration, Some(block_number));
                            }
                            Some(Ok((block_number, duration, Err(error)))) => {
                                active.remove(&block_number);
                                retry_after.insert(block_number, Instant::now() + BUILD_RETRY_DELAY);
                                scan_cursor = scan_cursor.min(block_number);
                                self.metrics.record_build("failure", duration, None);
                                tracing::error!(block_number, error = %error, "Failed to build SNOS block witness");
                            }
                            Some(Err(error)) => {
                                self.metrics.record_build("task_failure", Duration::ZERO, None);
                                tracing::error!(error = %error, "SNOS block witness task failed");
                            }
                            None => {}
                        }
                    }
                    _ = tokio::time::sleep(SCHEDULER_INTERVAL) => {}
                }
            }
        })
    }

    pub(crate) async fn ready_artifact(
        &self,
        block_number: u64,
    ) -> Result<(PathBuf, SnosWitnessManifest), SnosWitnessAvailabilityError> {
        let head = self.witnessable_head().ok_or(SnosWitnessAvailabilityError::NoWitnessableHead)?;
        let oldest = self.oldest_retained(head);
        if block_number < oldest {
            return Err(SnosWitnessAvailabilityError::OutsideRetention { block_number, oldest, head });
        }
        if block_number > head {
            return Err(SnosWitnessAvailabilityError::Pending { block_number });
        }
        let generation_floor = self.generation_floor(head)?;
        if block_number < generation_floor {
            return Err(SnosWitnessAvailabilityError::BeforeGenerationFloor { block_number, generation_floor });
        }

        self.ready_artifact_unchecked(block_number).await?.ok_or(SnosWitnessAvailabilityError::Pending { block_number })
    }

    pub(crate) async fn read_uncompressed(&self, path: PathBuf) -> anyhow::Result<Vec<u8>> {
        tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<u8>> {
            use std::io::Read as _;
            let file = std::fs::File::open(&path).with_context(|| format!("Opening {}", path.display()))?;
            let mut decoder = GzDecoder::new(file);
            let mut json = Vec::new();
            decoder.read_to_end(&mut json).with_context(|| format!("Reading {}", path.display()))?;
            Ok(json)
        })
        .await
        .context("Joining SNOS witness reader")?
    }

    pub(crate) async fn range_status(
        &self,
        start_block: u64,
        end_block: u64,
    ) -> Result<SnosWitnessRangeStatus, SnosWitnessAvailabilityError> {
        if start_block > end_block {
            return Err(SnosWitnessAvailabilityError::Internal(anyhow::anyhow!("startBlock must not exceed endBlock")));
        }
        let head = self.witnessable_head().ok_or(SnosWitnessAvailabilityError::NoWitnessableHead)?;
        let generation_floor = self.generation_floor(head)?;
        let oldest = self.oldest_retained(head);
        if start_block < oldest {
            return Err(SnosWitnessAvailabilityError::OutsideRetention { block_number: start_block, oldest, head });
        }

        let mut first_missing_block = None;
        for block_number in start_block..=end_block {
            if block_number < generation_floor
                || block_number > head
                || self.ready_artifact_unchecked(block_number).await?.is_none()
            {
                first_missing_block = Some(block_number);
                break;
            }
        }

        Ok(SnosWitnessRangeStatus {
            ready: first_missing_block.is_none(),
            start_block,
            end_block,
            witnessable_head: Some(head),
            oldest_retained_block: Some(oldest),
            generation_floor,
            first_missing_block,
        })
    }

    async fn next_missing_block(
        &self,
        scan_cursor: &mut u64,
        start: u64,
        target: u64,
        active: &BTreeSet<u64>,
        retry_after: &HashMap<u64, Instant>,
    ) -> Option<u64> {
        while *scan_cursor <= target {
            let block_number = *scan_cursor;
            *scan_cursor = scan_cursor.saturating_add(1);
            if active.contains(&block_number)
                || retry_after.get(&block_number).is_some_and(|retry| *retry > Instant::now())
            {
                continue;
            }
            match self.ready_artifact_unchecked(block_number).await {
                Ok(Some(_)) => continue,
                Ok(None) => return Some(block_number),
                Err(error) => {
                    tracing::warn!(block_number, %error, "Failed to inspect SNOS witness artifact");
                    return Some(block_number);
                }
            }
        }
        *scan_cursor = start;
        None
    }

    async fn ready_artifact_unchecked(
        &self,
        block_number: u64,
    ) -> anyhow::Result<Option<(PathBuf, SnosWitnessManifest)>> {
        let Some((block_hash, state_root)) = self.block_identity(block_number)? else {
            return Ok(None);
        };
        let path = self.witness_path(block_number, block_hash);
        let manifest_path = self.manifest_path(block_number, block_hash);
        if !tokio::fs::try_exists(&path).await? || !tokio::fs::try_exists(&manifest_path).await? {
            return Ok(None);
        }
        let manifest: SnosWitnessManifest = serde_json::from_slice(
            &tokio::fs::read(&manifest_path).await.with_context(|| format!("Reading {}", manifest_path.display()))?,
        )?;
        if manifest.block_number != block_number
            || manifest.block_hash != block_hash
            || manifest.state_root != state_root
        {
            return Ok(None);
        }
        Ok(Some((path, manifest)))
    }

    async fn build(&self, block_number: u64) -> anyhow::Result<()> {
        self.ensure_witnessable(block_number)?;
        let (block_hash, state_root) =
            self.block_identity(block_number)?.with_context(|| format!("Block {block_number} is not available"))?;
        if self.ready_artifact_unchecked(block_number).await?.is_some() {
            return Ok(());
        }

        let started_at = Instant::now();
        let witness = generate_pie::record_rpc_witness(self.pie_input(block_number)?)
            .await
            .with_context(|| format!("Recording SNOS witness for block {block_number}"))?;
        let schema_version = witness.schema_version();
        let response_count = witness.response_count();
        let final_path = self.witness_path(block_number, block_hash);
        let manifest_path = self.manifest_path(block_number, block_hash);
        let temporary_path = final_path.with_extension("gz.tmp");
        let temporary_manifest_path = manifest_path.with_extension("json.tmp");

        let temporary_path_for_writer = temporary_path.clone();
        let (compressed_bytes, uncompressed_bytes) = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
            let file = std::fs::File::create(&temporary_path_for_writer)
                .with_context(|| format!("Creating {}", temporary_path_for_writer.display()))?;
            let writer = std::io::BufWriter::new(file);
            let encoder = GzEncoder::new(writer, Compression::fast());
            let mut writer = CountingWriter::new(encoder);
            serde_json::to_writer(&mut writer, &witness).context("Serializing SNOS witness")?;
            let uncompressed_bytes = writer.written;
            writer.inner.finish().context("Finishing SNOS witness compression")?;
            let compressed_bytes = std::fs::metadata(&temporary_path_for_writer)?.len();
            Ok((compressed_bytes, uncompressed_bytes))
        })
        .await
        .context("Joining SNOS witness writer")??;

        self.ensure_witnessable(block_number)?;
        let current_identity = self.block_identity(block_number)?;
        if current_identity != Some((block_hash, state_root)) {
            let _ = tokio::fs::remove_file(&temporary_path).await;
            bail!("Block {block_number} changed while its SNOS witness was being built")
        }

        let manifest = SnosWitnessManifest {
            schema_version,
            block_number,
            block_hash,
            state_root,
            response_count,
            compressed_bytes,
            uncompressed_bytes,
        };
        tokio::fs::write(&temporary_manifest_path, serde_json::to_vec(&manifest)?)
            .await
            .with_context(|| format!("Writing {}", temporary_manifest_path.display()))?;
        tokio::fs::rename(&temporary_path, &final_path)
            .await
            .with_context(|| format!("Publishing {}", final_path.display()))?;
        tokio::fs::rename(&temporary_manifest_path, &manifest_path)
            .await
            .with_context(|| format!("Publishing {}", manifest_path.display()))?;
        self.prune().await?;
        tracing::info!(
            block_number,
            block_hash = %format_args!("{block_hash:#x}"),
            response_count,
            compressed_bytes,
            uncompressed_bytes,
            elapsed_ms = started_at.elapsed().as_millis(),
            path = %final_path.display(),
            "SNOS block witness created"
        );
        Ok(())
    }

    fn witnessable_head(&self) -> Option<u64> {
        witnessable_head(&self.backend)
    }

    fn oldest_retained(&self, head: u64) -> u64 {
        head.saturating_sub(self.config.retention_blocks.saturating_sub(1))
    }

    fn generation_floor(&self, current_head: u64) -> anyhow::Result<u64> {
        let mut generation_floor = self.generation_floor.lock().expect("SNOS witness generation floor lock poisoned");
        if let Some(generation_floor) = *generation_floor {
            return Ok(generation_floor);
        }
        let floor = create_generation_floor(&self.config.directory, current_head, self.config.initial_backfill_blocks)?;
        *generation_floor = Some(floor);
        tracing::info!(generation_floor = floor, "Initialized SNOS witness generation floor");
        Ok(floor)
    }

    fn ensure_witnessable(&self, block_number: u64) -> anyhow::Result<()> {
        let Some(head) = self.witnessable_head() else {
            bail!("Cannot build a witness before the first confirmed trie state")
        };
        if block_number > head {
            bail!("Block {block_number} is newer than trie-applied confirmed head {head}")
        }
        let oldest = self.oldest_retained(head);
        if block_number < oldest {
            bail!("Block {block_number} is outside the retained SNOS witness range {oldest}..={head}")
        }
        Ok(())
    }

    fn block_identity(&self, block_number: u64) -> anyhow::Result<Option<(Felt, Felt)>> {
        let Some(block) = self.backend.block_view_on_confirmed(block_number) else {
            return Ok(None);
        };
        let info = block.get_block_info()?;
        Ok(Some((info.block_hash, info.header.global_state_root)))
    }

    fn pie_input(&self, block_number: u64) -> anyhow::Result<PieGenerationInput> {
        let chain_info = self.backend.chain_config().blockifier_chain_info();
        let strk: Felt = chain_info.fee_token_addresses.strk_fee_token_address.into();
        let eth: Felt = chain_info.fee_token_addresses.eth_fee_token_address.into();
        Ok(PieGenerationInput {
            rpc_url: self.config.rpc_url.clone(),
            blocks: vec![block_number],
            layout: generate_pie::parse_layout("all_cairo")?,
            chain_config: ChainConfig::new(
                &chain_info.chain_id.to_string(),
                &format!("{strk:#x}"),
                &format!("{eth:#x}"),
                chain_info.is_l3,
            ),
            os_hints_config: OsHintsConfiguration::default_with_is_l3(chain_info.is_l3),
            output_path: None,
            versioned_constants: self.config.versioned_constants.clone(),
            public_keys: None,
        })
    }

    fn witness_path(&self, block_number: u64, block_hash: Felt) -> PathBuf {
        self.config.directory.join(format!("{block_number}-{block_hash:#x}.json.gz"))
    }

    fn manifest_path(&self, block_number: u64, block_hash: Felt) -> PathBuf {
        self.config.directory.join(format!("{block_number}-{block_hash:#x}.manifest.json"))
    }

    async fn prune(&self) -> anyhow::Result<()> {
        let Some(head) = self.witnessable_head() else { return Ok(()) };
        let oldest = self.oldest_retained(head);
        let mut entries = tokio::fs::read_dir(&self.config.directory).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if witness_block_number(&path).is_some_and(|block_number| block_number < oldest || block_number > head) {
                tokio::fs::remove_file(&path).await?;
            }
        }
        Ok(())
    }
}

fn witnessable_head(backend: &MadaraBackend) -> Option<u64> {
    backend
        .latest_confirmed_block_n()
        .zip(backend.get_latest_applied_trie_update().ok().flatten())
        .map(|(confirmed, trie_applied)| confirmed.min(trie_applied))
}

fn load_generation_floor(directory: &Path) -> anyhow::Result<Option<u64>> {
    let path = directory.join(GENERATION_FLOOR_FILE);
    match std::fs::read_to_string(&path) {
        Ok(value) => value.trim().parse().map(Some).with_context(|| format!("Parsing {}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("Reading {}", path.display())),
    }
}

fn create_generation_floor(directory: &Path, current_head: u64, initial_backfill_blocks: u64) -> anyhow::Result<u64> {
    let path = directory.join(GENERATION_FLOOR_FILE);
    let floor = match initial_backfill_blocks {
        0 => current_head.saturating_add(1),
        count => current_head.saturating_sub(count.saturating_sub(1)),
    };
    let temporary = path.with_extension("tmp");
    std::fs::write(&temporary, floor.to_string()).with_context(|| format!("Writing {}", temporary.display()))?;
    std::fs::rename(&temporary, &path).with_context(|| format!("Publishing {}", path.display()))?;
    Ok(floor)
}

struct CountingWriter<W> {
    inner: W,
    written: u64,
}

impl<W> CountingWriter<W> {
    fn new(inner: W) -> Self {
        Self { inner, written: 0 }
    }
}

impl<W: Write> Write for CountingWriter<W> {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        let written = self.inner.write(buffer)?;
        self.written = self.written.saturating_add(written as u64);
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

fn witness_block_number(path: &Path) -> Option<u64> {
    let name = path.file_name()?.to_str()?;
    name.split_once('-')
        .and_then(|(block_number, _)| block_number.parse().ok())
        .or_else(|| name.strip_suffix(".json.gz")?.parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_versioned_and_legacy_witness_artifacts_for_pruning() {
        assert_eq!(witness_block_number(Path::new("123-0xabc.json.gz")), Some(123));
        assert_eq!(witness_block_number(Path::new("123-0xabc.manifest.json")), Some(123));
        assert_eq!(witness_block_number(Path::new("123.json.gz")), Some(123));
        assert_eq!(witness_block_number(Path::new(".generation-floor")), None);
    }

    #[test]
    fn generation_floor_is_durable() {
        let directory = tempfile::tempdir().unwrap();
        assert_eq!(load_generation_floor(directory.path()).unwrap(), None);
        assert_eq!(create_generation_floor(directory.path(), 100, 10).unwrap(), 91);
        assert_eq!(load_generation_floor(directory.path()).unwrap(), Some(91));
    }
}
