use crate::{
    apply_state::ApplyStateSync,
    import::BlockImporter,
    metrics::SyncMetrics,
    probe::ThrottledRepeatedFuture,
    reorg::{detect_reorg},
    sync::{ForwardPipeline, SyncController, SyncControllerConfig},
};
use anyhow::Context;
use blocks::{gateway_preconfirmed_block_sync, GatewayBlockSync};
use classes::ClassesSync;
use mc_db::MadaraBackend;
use mc_gateway_client::GatewayProvider;
use mp_block::{BlockId, BlockTag};
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
    pub keep_pre_v0_13_2_hashes: bool,
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
            keep_pre_v0_13_2_hashes: false,
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
}

pub type GatewaySync = SyncController<GatewayForwardSync>;
pub async fn forward_sync(
    backend: Arc<MadaraBackend>,
    importer: Arc<BlockImporter>,
    client: Arc<GatewayProvider>,
    controller_config: SyncControllerConfig,
    config: ForwardSyncConfig,
) -> anyhow::Result<GatewaySync> {
    let probe = Arc::new(GatewayLatestProbe::new(client.clone()));
    let probe = ThrottledRepeatedFuture::new(move |val| probe.clone().probe(val), Duration::from_secs(1));
    let get_pending_block = gateway_preconfirmed_block_sync(client.clone(), importer.clone(), backend.clone());
    let forward_sync = GatewayForwardSync::new(backend.clone(), importer, client, config).await?;
    Ok(SyncController::new(
        backend.clone(),
        forward_sync,
        probe,
        controller_config,
        Some(get_pending_block),
    ))
}

pub struct GatewayForwardSync {
    blocks_pipeline: GatewayBlockSync,
    classes_pipeline: ClassesSync,
    apply_state_pipeline: ApplyStateSync,
    backend: Arc<MadaraBackend>,
    client: Arc<GatewayProvider>,
    importer: Arc<BlockImporter>,
    config: ForwardSyncConfig,
}

impl GatewayForwardSync {
    pub async fn new(
        backend: Arc<MadaraBackend>,
        importer: Arc<BlockImporter>,
        client: Arc<GatewayProvider>,
        config: ForwardSyncConfig,
    ) -> anyhow::Result<Self> {
        tracing::info!("🌐 Initializing sync from gateway");

        let mut starting_block_n = backend.latest_confirmed_block_n().map(|n| n + 1).unwrap_or(0);

        tracing::info!("🚀 GatewayForwardSync::new - latest_confirmed_block_n: {:?}, starting_block_n: {}",
                      backend.latest_confirmed_block_n(), starting_block_n);

        // If we have local blocks, check if we need to reorg before starting
        if let Some(latest_block) = backend.latest_confirmed_block_n() {
            tracing::info!("📊 Found local blocks, latest: {}", latest_block);
            if latest_block > 0 {
                tracing::info!("🔍 Checking for chain divergence before sync starts (latest local block: {})", latest_block);

                // Check our latest block against the gateway
                if let Some(common_ancestor) = detect_reorg(
                    latest_block,
                    &backend,
                    &client,
                ).await? {
                    tracing::warn!("⚠️ REORG DETECTED at initialization - common ancestor: {}", common_ancestor);
                    tracing::warn!("🔄 Rolling back from block {} to block {}", latest_block, common_ancestor);

                    // Perform rollback
                    backend.rollback_to_block(common_ancestor)?;

                    // Update starting block after rollback
                    starting_block_n = common_ancestor + 1;

                    tracing::info!("✅ Rollback complete. Sync will start from block {}", starting_block_n);
                } else {
                    tracing::info!("✅ No chain divergence detected. Continuing from block {}", starting_block_n);
                }
            }
        }

        let blocks_pipeline = blocks::block_with_state_update_pipeline(
            backend.clone(),
            importer.clone(),
            client.clone(),
            starting_block_n,
            config.block_parallelization,
            config.block_batch_size,
            config.keep_pre_v0_13_2_hashes,
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
        );
        Ok(Self { blocks_pipeline, classes_pipeline, apply_state_pipeline, backend, client, importer, config })
    }

    fn reinit_pipelines_from_current_position(&mut self) {
        // Reinitialize pipelines from the current database position
        let starting_block_n = self.backend.latest_confirmed_block_n().map(|n| n + 1).unwrap_or(0);
        tracing::info!("📊 Reinitializing pipelines from block #{} after rollback", starting_block_n);

        self.blocks_pipeline = blocks::block_with_state_update_pipeline(
            self.backend.clone(),
            self.importer.clone(),
            self.client.clone(),
            starting_block_n,
            self.config.block_parallelization,
            self.config.block_batch_size,
            self.config.keep_pre_v0_13_2_hashes,
        );
        self.classes_pipeline = classes::classes_pipeline(
            self.backend.clone(),
            self.importer.clone(),
            self.client.clone(),
            starting_block_n,
            self.config.classes_parallelization,
            self.config.classes_batch_size,
        );
        self.apply_state_pipeline = super::apply_state::apply_state_pipeline(
            self.backend.clone(),
            self.importer.clone(),
            starting_block_n,
            self.config.apply_state_parallelization,
            self.config.apply_state_batch_size,
            self.config.disable_tries,
        );
    }

    fn pipeline_status(&self) -> PipelineStatus {
        PipelineStatus {
            blocks: self.blocks_pipeline.last_applied_block_n(),
            classes: self.classes_pipeline.last_applied_block_n(),
            apply_state: self.apply_state_pipeline.last_applied_block_n(),
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

        let mut done = false;

        // Reorg detection now happens in the constructor, before pipelines are initialized
        // This ensures we don't start syncing with the wrong chain

        while !done {
            while self.blocks_pipeline.can_schedule_more() && self.blocks_pipeline.next_input_block_n() <= target_height
            {
                let next_input_block_n = self.blocks_pipeline.next_input_block_n();

                // Check for reorg before scheduling each block during sync
                // Only check blocks we haven't synced yet but are within our stored range
                if let Some(latest_block) = self.backend.latest_confirmed_block_n() {
                    if next_input_block_n <= latest_block {
                        if let Some(common_ancestor) = detect_reorg(
                            next_input_block_n,
                            &self.backend,
                            &self.client,
                        ).await? {
                            // Reorg detected during sync!
                            tracing::warn!("⚠️ REORG DETECTED during sync at block {} - common ancestor: {}", next_input_block_n, common_ancestor);
                            tracing::warn!("🔄 Switching from previous chain to new chain from gateway");

                            // Perform rollback
                            self.backend.rollback_to_block(common_ancestor)?;

                            tracing::info!("✅ Rollback complete. Database now at block {}. Sync will continue from block {}",
                                common_ancestor, common_ancestor + 1);

                            // Reinitialize pipelines from the new position after rollback
                            self.reinit_pipelines_from_current_position();

                            // Continue with the sync from the new position
                            // Don't return - let the loop continue with the updated pipelines
                        }
                    }
                }

                self.blocks_pipeline.push(next_input_block_n..next_input_block_n + 1, iter::once(()));
            }

            let start_next_block = self.pipeline_status().min().map(|n| n + 1).unwrap_or(0);

            tokio::select! {
                Some(res) = self.apply_state_pipeline.next() => {
                    // Check if we got a GlobalStateRoot mismatch error which might indicate a reorg
                    if let Err(err) = &res {
                        let err_msg = format!("{:?}", err);
                        // Check both the error message and its cause for state root mismatch
                        if err_msg.contains("Global state root mismatch") || err_msg.contains("GlobalStateRoot") {
                            tracing::warn!("🔄 State root mismatch detected during apply_state - checking for potential reorg");
                            tracing::debug!("Error details: {}", err);

                            // Try to extract the block number from the error context
                            if let Some(latest_block) = self.backend.latest_confirmed_block_n() {
                                if let Some(common_ancestor) = detect_reorg(
                                    latest_block,
                                    &self.backend,
                                    &self.client,
                                ).await? {
                                    tracing::warn!("⚠️ REORG DETECTED via state root mismatch - common ancestor: {}", common_ancestor);
                                    tracing::warn!("🔄 Rolling back from block {} to block {}", latest_block, common_ancestor);

                                    // Perform rollback
                                    self.backend.rollback_to_block(common_ancestor)?;

                                    // Reinitialize pipelines from the new position after rollback
                                    self.reinit_pipelines_from_current_position();

                                    tracing::info!("✅ Rollback complete. Sync will continue from block {}", common_ancestor + 1);

                                    // Continue the loop without propagating the error
                                    continue;
                                } else {
                                    tracing::warn!("⚠️ State root mismatch but no reorg detected - this might be a different issue");
                                }
                            }
                        }
                    }
                    res?;
                }
                Some(res) = self.classes_pipeline.next() => {
                    res?;
                }
                Some(res) = self.blocks_pipeline.next(), if self.classes_pipeline.can_schedule_more() && self.apply_state_pipeline.can_schedule_more() => {
                    let (range, state_diffs) = res?;
                    self.classes_pipeline.push(range.clone(), state_diffs.iter().map(|s| s.all_declared_classes()));
                    self.apply_state_pipeline.push(range, state_diffs);
                }
                // all pipelines are empty, we're done :)
                else => done = true,
            }

            let new_next_block = self.pipeline_status().min().map(|n| n + 1).unwrap_or(0);
            for block_n in start_next_block..new_next_block {
                tracing::info!("📊 Block #{} has passed through all 3 pipelines (blocks, classes, state) - marking as fully imported", block_n);

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
            self.apply_state_pipeline.status(),
        );
        let status = self.pipeline_status();
        if let Some(min_block) = status.min() {
            tracing::debug!(
                "Pipeline positions - Blocks: {:?}, Classes: {:?}, State: {:?}, Min (fully processed): {}",
                status.blocks, status.classes, status.apply_state, min_block
            );
        }
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
        _highest_known_block: Option<ProviderBlockHeader>,
    ) -> anyhow::Result<Option<ProviderBlockHeader>> {
        let header = self
            .client
            .get_header(BlockId::Tag(BlockTag::Latest))
            .await
            .context("Getting the latest block_n from the gateway")?;
        tracing::debug!("Probe got header {header:?}");
        Ok(Some(header))
    }
}
