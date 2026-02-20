use crate::{
    apply_state::ApplyStateSync,
    import::BlockImporter,
    metrics::SyncMetrics,
    probe::ThrottledRepeatedFuture,
    sync::{ForwardPipeline, SyncController, SyncControllerConfig},
};
use anyhow::Context;
use blocks::{gateway_preconfirmed_block_sync, GatewayBlockSync};
use classes::ClassesSync;
use mc_db::{MadaraBackend, MadaraStorageRead};
use mc_gateway_client::{BlockId, BlockTag, GatewayProvider};
use mp_gateway::block::ProviderBlockHeader;
use std::{iter, sync::Arc, time::Duration};

pub(crate) mod blocks;
pub(crate) mod classes;

pub struct ForwardSyncConfig {
    pub block_parallelization: usize,
    pub block_batch_size: usize,
    pub classes_parallelization: usize,
    pub classes_batch_size: usize,
    pub apply_state_parallelization: usize,
    pub apply_state_batch_size: usize,
    pub disable_tries: bool,
    pub snap_sync: bool,
    pub keep_pre_v0_13_2_hashes: bool,
    pub enable_bouncer_config_sync: bool,
    pub disable_reorg: bool,
}

impl Default for ForwardSyncConfig {
    fn default() -> Self {
        Self {
            block_parallelization: 128,
            block_batch_size: 1,
            classes_parallelization: 256,
            classes_batch_size: 1,
            apply_state_parallelization: 16,
            apply_state_batch_size: 4,
            disable_tries: false,
            snap_sync: false,
            keep_pre_v0_13_2_hashes: false,
            enable_bouncer_config_sync: false,
            disable_reorg: false,
        }
    }
}

impl ForwardSyncConfig {
    pub fn disable_tries(self, val: bool) -> Self {
        Self { disable_tries: val, ..self }
    }
    pub fn keep_pre_v0_13_2_hashes(self, val: bool) -> Self {
        Self { keep_pre_v0_13_2_hashes: val, ..self }
    }
    pub fn snap_sync(self, val: bool) -> Self {
        Self { snap_sync: val, ..self }
    }

    pub fn enable_bouncer_config_sync(self, val: bool) -> Self {
        Self { enable_bouncer_config_sync: val, ..self }
    }
    pub fn disable_reorg(self, val: bool) -> Self {
        Self { disable_reorg: val, ..self }
    }
}

pub type GatewaySync = SyncController<GatewayForwardSync>;
pub fn forward_sync(
    backend: Arc<MadaraBackend>,
    importer: Arc<BlockImporter>,
    client: Arc<GatewayProvider>,
    controller_config: SyncControllerConfig,
    config: ForwardSyncConfig,
) -> GatewaySync {
    let probe = Arc::new(GatewayLatestProbe::new(client.clone()));
    let probe = ThrottledRepeatedFuture::new(move |val| probe.clone().probe(val), Duration::from_secs(1));
    let get_pending_block = gateway_preconfirmed_block_sync(client.clone(), importer.clone(), backend.clone());
    SyncController::new(
        backend.clone(),
        GatewayForwardSync::new(backend, importer, client, config),
        probe,
        controller_config,
        Some(get_pending_block),
    )
}

pub struct GatewayForwardSync {
    blocks_pipeline: GatewayBlockSync,
    classes_pipeline: ClassesSync,
    apply_state_pipeline: ApplyStateSync,
    backend: Arc<MadaraBackend>,
    importer: Arc<BlockImporter>,
    client: Arc<GatewayProvider>,
    config: ForwardSyncConfig,
}

impl GatewayForwardSync {
    /// Helper function to create all three pipelines from a starting block number
    fn create_pipelines(
        backend: Arc<MadaraBackend>,
        importer: Arc<BlockImporter>,
        client: Arc<GatewayProvider>,
        starting_block_n: u64,
        config: &ForwardSyncConfig,
    ) -> (GatewayBlockSync, ClassesSync, ApplyStateSync) {
        let blocks_pipeline = blocks::block_with_state_update_pipeline(
            backend.clone(),
            importer.clone(),
            client.clone(),
            starting_block_n,
            config.block_parallelization,
            config.block_batch_size,
            config.keep_pre_v0_13_2_hashes,
            config.enable_bouncer_config_sync,
            config.disable_reorg,
        );
        let classes_pipeline = classes::classes_pipeline(
            backend.clone(),
            importer.clone(),
            client.clone(),
            starting_block_n,
            config.classes_parallelization,
            config.classes_batch_size,
        );
        let apply_state_pipeline = super::apply_state::apply_state_pipeline(
            backend.clone(),
            importer.clone(),
            starting_block_n,
            config.apply_state_parallelization,
            config.apply_state_batch_size,
            config.disable_tries,
            config.snap_sync,
        );
        (blocks_pipeline, classes_pipeline, apply_state_pipeline)
    }

    pub fn new(
        backend: Arc<MadaraBackend>,
        importer: Arc<BlockImporter>,
        client: Arc<GatewayProvider>,
        config: ForwardSyncConfig,
    ) -> Self {
        tracing::info!("🌐 Initializing sync from gateway");

        let latest_full_block = backend.latest_block_n().unwrap_or(0);
        let starting_block_n = backend.latest_confirmed_block_n().map(|n| n + 1).unwrap_or(0);
        tracing::info!(
            "📊 Database head status: latest_full_block={:?}, starting sync from block #{}",
            latest_full_block,
            starting_block_n
        );

        let (blocks_pipeline, classes_pipeline, apply_state_pipeline) =
            Self::create_pipelines(backend.clone(), importer.clone(), client.clone(), starting_block_n, &config);

        Self {
            blocks_pipeline,
            classes_pipeline,
            apply_state_pipeline,
            backend,
            importer: importer.clone(),
            client: client.clone(),
            config,
        }
    }

    /// Reinitialize pipelines from the database's current tip
    /// This is used after a reorg to restart from the new chain head
    pub fn reinit_pipelines(&mut self) {
        tracing::info!("🔄 Reinitializing pipelines after reorg");
        // Ensure we flush any pending writes before reading head status
        if let Err(e) = self.backend.db.flush() {
            tracing::warn!("Failed to flush database before reading head status: {}", e);
        }

        // After reorg, read chain tip directly from database to get fresh value
        // (the cached chain_tip may be stale after revert_to)
        let chain_tip = self.backend.db.get_chain_tip().expect("Failed to get chain tip after reorg");
        let starting_block_n = match chain_tip {
            mc_db::storage::StorageChainTip::Confirmed(block_n) => block_n + 1,
            mc_db::storage::StorageChainTip::Preconfirmed { .. } => {
                tracing::warn!("Unexpected preconfirmed block after reorg");
                0
            }
            mc_db::storage::StorageChainTip::Empty => 0,
        };
        tracing::info!("📊 Restarting sync from block #{} (chain_tip={:?})", starting_block_n, chain_tip);

        let (blocks_pipeline, classes_pipeline, apply_state_pipeline) = Self::create_pipelines(
            self.backend.clone(),
            self.importer.clone(),
            self.client.clone(),
            starting_block_n,
            &self.config,
        );

        self.blocks_pipeline = blocks_pipeline;
        self.classes_pipeline = classes_pipeline;
        self.apply_state_pipeline = apply_state_pipeline;
    }

    fn pipeline_status(&self) -> PipelineStatus {
        PipelineStatus {
            blocks: self.blocks_pipeline.last_applied_block_n(),
            classes: self.classes_pipeline.last_applied_block_n(),
            apply_state: self.backend.get_latest_applied_trie_update().ok().flatten(),
        }
    }
}

#[derive(Clone)]
struct PipelineStatus {
    blocks: Option<u64>,
    classes: Option<u64>,
    apply_state: Option<u64>,
}

impl PipelineStatus {
    pub fn min(&self) -> Option<u64> {
        self.blocks.min(self.classes).min(self.apply_state)
    }
}

impl ForwardPipeline for GatewayForwardSync {
    async fn run(
        &mut self,
        target_height: u64,
        _probe_height: Option<u64>,
        metrics: &mut SyncMetrics,
    ) -> anyhow::Result<()> {
        tracing::debug!("Run pipeline to height={target_height:?}");

        self.apply_state_pipeline.set_target_block(target_height);

        let mut done = false;
        while !done {
            while self.blocks_pipeline.can_schedule_more() && self.blocks_pipeline.next_input_block_n() <= target_height
            {
                let next_input_block_n = self.blocks_pipeline.next_input_block_n();

                self.blocks_pipeline.push(next_input_block_n..next_input_block_n + 1, iter::once(()));
            }

            tokio::select! {
                Some(res) = self.apply_state_pipeline.next() => {
                    res?;
                }
                Some(res) = self.classes_pipeline.next() => {
                    res?;
                }
                Some(res) = self.blocks_pipeline.next(), if self.classes_pipeline.can_schedule_more() && self.apply_state_pipeline.can_schedule_more() => {
                    match res {
                        Ok((range, state_diffs)) => {
                            self.classes_pipeline.push(range.clone(), state_diffs.iter().map(|s| s.all_declared_classes()));
                            self.apply_state_pipeline.push(range, state_diffs);
                        }
                        Err(e) => {
                            // Check if this is a reorg error or genesis mismatch recovery
                            let error_msg = e.to_string();
                            if error_msg.contains("Reorg detected and processed") {
                                tracing::info!("🔄 Handling reorg: reinitializing all pipelines from new chain tip");
                                self.reinit_pipelines();
                                // Break inner loop to restart with fresh pipeline instances
                                // Returning Ok() causes the outer run loop to call us again with new pipelines
                                break;
                            } else if error_msg.contains("Genesis mismatch resolved - database cleared") {
                                tracing::info!("🔄 Handling genesis mismatch recovery: reinitializing all pipelines from empty database");
                                self.reinit_pipelines();
                                // Pipeline will now start from block 0 (empty database)
                                // Will fetch correct genesis from upstream
                                break;
                            } else {
                                // Propagate other errors
                                return Err(e);
                            }
                        }
                    }
                }
                // all pipelines are empty, we're done :)
                else => done = true,
            }

            // Calculate the range of blocks to seal based on:
            // - start_next_block: The block after the current chain tip (last sealed block)
            // - new_next_block: The block after the minimum of all pipeline completions
            // This ensures we only seal blocks that have been fully processed by ALL pipelines
            let start_next_block = self.backend.latest_confirmed_block_n().map(|n| n + 1).unwrap_or(0);
            let new_next_block = self.pipeline_status().min().map(|n| n + 1).unwrap_or(0);

            for block_n in start_next_block..new_next_block {
                // Mark the block as fully imported.
                self.backend.write_access().new_confirmed_block(block_n)?;
                metrics.update(block_n, &self.backend).context("Updating metrics")?;
            }
        }

        Ok(())
    }

    fn next_input_block_n(&self) -> u64 {
        self.blocks_pipeline.next_input_block_n()
    }

    fn is_empty(&self) -> bool {
        self.blocks_pipeline.is_empty() && self.classes_pipeline.is_empty() && self.apply_state_pipeline.is_empty()
    }

    fn show_status(&self) {
        tracing::info!(
            "📥 Blocks: {} | Classes: {} | State: {}",
            self.blocks_pipeline.status(),
            self.classes_pipeline.status(),
            self.apply_state_pipeline.trie_state_status(),
        );
    }

    fn latest_block(&self) -> Option<u64> {
        self.backend.latest_confirmed_block_n()
    }
}

struct GatewayLatestProbe {
    client: Arc<GatewayProvider>,
}

impl GatewayLatestProbe {
    pub fn new(client: Arc<GatewayProvider>) -> Self {
        Self { client }
    }

    async fn probe(
        self: Arc<Self>,
        highest_known_block: Option<ProviderBlockHeader>,
    ) -> anyhow::Result<Option<ProviderBlockHeader>> {
        match self.client.get_header(BlockId::Tag(BlockTag::Latest)).await {
            Ok(header) => {
                tracing::debug!("Probe got header {header:?}");
                Ok(Some(header))
            }
            Err(e) => {
                // Log the error but don't crash - gateway being temporarily unavailable
                // should not stop the sync service. We'll return the last known block
                // and retry on the next probe cycle.
                tracing::warn!(
                    "Failed to get latest block from gateway: {e:#}. \
                     Sync will continue with last known block height. \
                     Will retry on next probe cycle."
                );
                // Return the last known block if we have one, otherwise None
                // This allows the sync to continue operating with cached data
                Ok(highest_known_block)
            }
        }
    }
}
