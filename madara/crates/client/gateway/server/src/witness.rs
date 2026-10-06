use anyhow::{bail, Context};
use blockifier::blockifier_versioned_constants::VersionedConstants;
use flate2::{read::GzDecoder, write::GzEncoder, Compression};
use generate_pie::types::{ChainConfig, OsHintsConfiguration, PieGenerationInput};
use mc_db::MadaraBackend;
use starknet_types_core::felt::Felt;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::{Mutex, Semaphore};

const RETENTION_ENV: &str = "MADARA_SNOS_WITNESS_RETENTION_BLOCKS";
const DIRECTORY_ENV: &str = "MADARA_SNOS_WITNESS_DIR";
const RPC_URL_ENV: &str = "MADARA_SNOS_WITNESS_RPC_URL";
const BUILD_CONCURRENCY_ENV: &str = "MADARA_SNOS_WITNESS_BUILD_CONCURRENCY";
const VERSIONED_CONSTANTS_PATH_ENV: &str = "MADARA_SNOS_WITNESS_VERSIONED_CONSTANTS_PATH";
const DEFAULT_DIRECTORY: &str = "/data/snos-witnesses";
const DEFAULT_RPC_URL: &str = "http://127.0.0.1:9944";

#[derive(Debug, Clone)]
struct SnosWitnessConfig {
    retention_blocks: u64,
    directory: PathBuf,
    rpc_url: String,
    build_concurrency: usize,
    versioned_constants: Option<VersionedConstants>,
}

#[derive(Debug)]
pub(crate) struct SnosWitnessService {
    backend: Arc<MadaraBackend>,
    config: SnosWitnessConfig,
    build_slots: Semaphore,
    block_locks: std::sync::Mutex<HashMap<u64, Arc<Mutex<()>>>>,
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

        let config = SnosWitnessConfig {
            retention_blocks,
            directory: std::env::var(DIRECTORY_ENV).unwrap_or_else(|_| DEFAULT_DIRECTORY.to_owned()).into(),
            rpc_url: std::env::var(RPC_URL_ENV).unwrap_or_else(|_| DEFAULT_RPC_URL.to_owned()),
            build_concurrency: std::env::var(BUILD_CONCURRENCY_ENV)
                .unwrap_or_else(|_| "2".to_owned())
                .parse::<usize>()
                .with_context(|| format!("Parsing {BUILD_CONCURRENCY_ENV}"))?,
            versioned_constants: generate_pie::utils::load_versioned_constants(
                std::env::var(VERSIONED_CONSTANTS_PATH_ENV).ok().as_deref(),
            )
            .map_err(anyhow::Error::msg)?,
        };
        if config.build_concurrency == 0 {
            bail!("{BUILD_CONCURRENCY_ENV} must be greater than zero")
        }
        std::fs::create_dir_all(&config.directory)
            .with_context(|| format!("Creating SNOS witness directory {}", config.directory.display()))?;
        tracing::info!(
            retention_blocks,
            directory = %config.directory.display(),
            rpc_url = %config.rpc_url,
            build_concurrency = config.build_concurrency,
            "SNOS block witness generation enabled"
        );

        let build_concurrency = config.build_concurrency;
        Ok(Some(Arc::new(Self {
            backend,
            config,
            build_slots: Semaphore::new(build_concurrency),
            block_locks: std::sync::Mutex::new(HashMap::new()),
        })))
    }

    pub(crate) fn spawn_head_builder(self: Arc<Self>) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut heads = self.backend.watch_chain_head_state();
            let mut last_seen = heads.current().confirmed_tip;
            let mut pending = None;
            let mut builds = tokio::task::JoinSet::new();

            loop {
                while builds.len() < self.config.build_concurrency {
                    let Some(block_number) = pending.take() else { break };
                    let service = Arc::clone(&self);
                    builds.spawn(async move {
                        let result = service.ensure_exists(block_number).await;
                        (block_number, result)
                    });
                }

                tokio::select! {
                    state = heads.recv() => {
                        let Some(current) = state.confirmed_tip else {
                            last_seen = None;
                            continue;
                        };
                        if last_seen.is_none_or(|last| current > last) {
                            // Keep only the newest unscheduled head. This prevents restored or
                            // temporarily slow nodes from creating an unbounded historical build
                            // queue; retained gaps are still generated lazily on request.
                            pending = Some(current);
                        }
                        last_seen = Some(current);
                    }
                    joined = builds.join_next(), if !builds.is_empty() => {
                        match joined {
                            Some(Ok((block_number, result))) => {
                                if let Err(error) = result {
                                    tracing::error!(block_number, error = %error, "Failed to build SNOS block witness");
                                }
                            }
                            Some(Err(error)) => {
                                tracing::error!(error = %error, "SNOS block witness task failed");
                            }
                            None => {}
                        }
                    }
                }
            }
        })
    }

    pub(crate) async fn get_or_create_json(&self, block_number: u64) -> anyhow::Result<String> {
        let path = self.ensure_exists(block_number).await?;
        tokio::task::spawn_blocking(move || -> anyhow::Result<String> {
            use std::io::Read as _;
            let file = std::fs::File::open(&path).with_context(|| format!("Opening {}", path.display()))?;
            let mut decoder = GzDecoder::new(file);
            let mut json = String::new();
            decoder.read_to_string(&mut json).with_context(|| format!("Reading {}", path.display()))?;
            Ok(json)
        })
        .await
        .context("Joining SNOS witness reader")?
    }

    async fn ensure_exists(&self, block_number: u64) -> anyhow::Result<PathBuf> {
        self.ensure_retained(block_number)?;
        let path = self.witness_path(block_number);
        if tokio::fs::try_exists(&path).await? {
            return Ok(path);
        }

        let block_lock = {
            let mut locks = self.block_locks.lock().expect("SNOS witness block lock map poisoned");
            Arc::clone(locks.entry(block_number).or_insert_with(|| Arc::new(Mutex::new(()))))
        };
        let _block_guard = block_lock.lock().await;
        self.ensure_retained(block_number)?;
        if tokio::fs::try_exists(&path).await? {
            return Ok(path);
        }
        let _build_slot = self.build_slots.acquire().await.context("SNOS witness build semaphore closed")?;

        let started_at = std::time::Instant::now();
        let input = self.pie_input(block_number)?;
        let witness = generate_pie::record_rpc_witness(input)
            .await
            .with_context(|| format!("Recording SNOS witness for block {block_number}"))?;
        let response_count = witness.response_count();
        let final_path = path.clone();
        let temporary_path = path.with_extension("gz.tmp");
        tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
            let file = std::fs::File::create(&temporary_path)
                .with_context(|| format!("Creating {}", temporary_path.display()))?;
            let writer = std::io::BufWriter::new(file);
            let mut encoder = GzEncoder::new(writer, Compression::fast());
            serde_json::to_writer(&mut encoder, &witness).context("Serializing SNOS witness")?;
            encoder.finish().context("Finishing SNOS witness compression")?;
            std::fs::rename(&temporary_path, &final_path)
                .with_context(|| format!("Publishing {}", final_path.display()))?;
            Ok(())
        })
        .await
        .context("Joining SNOS witness writer")??;
        self.prune().await?;
        tracing::info!(
            block_number,
            response_count,
            elapsed_ms = started_at.elapsed().as_millis(),
            path = %path.display(),
            "SNOS block witness created"
        );
        Ok(path)
    }

    fn ensure_retained(&self, block_number: u64) -> anyhow::Result<()> {
        let Some(head) = self.backend.latest_confirmed_block_n() else {
            bail!("Cannot build a witness before the first confirmed block")
        };
        if block_number > head {
            bail!("Block {block_number} is newer than confirmed head {head}")
        }
        let oldest = head.saturating_sub(self.config.retention_blocks.saturating_sub(1));
        if block_number < oldest {
            bail!("Block {block_number} is outside the retained SNOS witness range {oldest}..={head}")
        }
        Ok(())
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

    fn witness_path(&self, block_number: u64) -> PathBuf {
        self.config.directory.join(format!("{block_number}.json.gz"))
    }

    async fn prune(&self) -> anyhow::Result<()> {
        let Some(head) = self.backend.latest_confirmed_block_n() else { return Ok(()) };
        let oldest = head.saturating_sub(self.config.retention_blocks.saturating_sub(1));
        let mut entries = tokio::fs::read_dir(&self.config.directory).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if witness_block_number(&path).is_some_and(|block_number| block_number < oldest) {
                tokio::fs::remove_file(&path).await?;
            }
        }
        self.block_locks
            .lock()
            .expect("SNOS witness block lock map poisoned")
            .retain(|block_number, _| *block_number >= oldest);
        Ok(())
    }
}

fn witness_block_number(path: &Path) -> Option<u64> {
    path.file_name()?.to_str()?.strip_suffix(".json.gz")?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_only_completed_witness_files() {
        assert_eq!(witness_block_number(Path::new("123.json.gz")), Some(123));
        assert_eq!(witness_block_number(Path::new("123.json.gz.tmp")), None);
        assert_eq!(witness_block_number(Path::new("not-a-block.json.gz")), None);
    }
}
