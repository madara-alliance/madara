use crate::{metrics::SyncMetrics, probe::ThrottledRepeatedFuture, util::ServiceStateSender};
use futures::{future::OptionFuture, Future};
use mc_db::sync_status::SyncStatus;
use mc_db::{MadaraBackend, MadaraStorageRead};
use mc_settlement_client::state_update::{L1HeadReceiver, StateUpdate};
use mp_gateway::block::ProviderBlockHeader;
use std::sync::Arc;
use std::{cmp, time::Duration};
use anyhow::anyhow;
use mp_block::BlockId;
use tokio::time::Instant;
use mc_db::storage::StorageChainTip;
use crate::sync_utils::{compress_state_diff, StateDiffMap};

pub trait ForwardPipeline {
    fn run(
        &mut self,
        target_block_n: u64,
        probe_height: Option<u64>,
        metrics: &mut SyncMetrics,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;
    fn next_input_block_n(&self) -> u64;
    fn show_status(&self);
    /// Return false when no work can be done.
    fn is_empty(&self) -> bool;
    fn latest_block(&self) -> Option<u64>;
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum ServiceEvent {
    Starting,
    UpdatedPreconfirmedBlock,
    Idle,
    SyncingTo { target: u64 },
}

pub struct SyncControllerConfig {
    pub l1_head_recv: L1HeadReceiver,
    /// Stop the sync process at this block.
    pub stop_at_block_n: Option<u64>,
    /// Call [`mp_utils::service::ServiceContext::cancel_global`] when the sync process finishes.
    /// This usually means that the whole node will be stopped
    pub global_stop_on_sync: bool,
    /// Disable syncing the pending block.
    pub no_pending_block: bool,
    /// Stop the service once fully synced, meaning the pipeline cannot be run again, and the probe did not return
    /// any new block - or the sync arrived at the block_n specified by [`Self::stop_at_block_n`].
    /// By default, the sync process will not stop, and pending block task / the probe will continue to run, even if
    /// [`Self::stop_at_block_n`] is set.
    pub stop_on_sync: bool,

    /// For testing purposes, you can subscribe to the service state. This is used in tests
    /// to know when the service is idling.
    pub service_state_sender: ServiceStateSender<ServiceEvent>,
}

impl SyncControllerConfig {
    pub fn stop_on_sync(self, stop_on_sync: bool) -> Self {
        Self { stop_on_sync, ..self }
    }
    pub fn l1_head_recv(self, l1_head_recv: L1HeadReceiver) -> Self {
        Self { l1_head_recv, ..self }
    }
    pub fn stop_at_block_n(self, stop_at_block_n: Option<u64>) -> Self {
        Self { stop_at_block_n, ..self }
    }
    pub fn global_stop_on_sync(self, global_stop_on_sync: bool) -> Self {
        Self { global_stop_on_sync, ..self }
    }
    pub fn no_pending_block(self, no_pending_block: bool) -> Self {
        Self { no_pending_block, ..self }
    }
    pub fn service_state_sender(self, service_state_sender: ServiceStateSender<ServiceEvent>) -> Self {
        Self { service_state_sender, ..self }
    }
}

impl Default for SyncControllerConfig {
    fn default() -> Self {
        // Make a channel that has its sender closed. No notification can happen on this channel.
        let (_, l1_head_recv) = tokio::sync::watch::channel(None);
        Self {
            l1_head_recv,
            stop_at_block_n: None,
            global_stop_on_sync: false,
            stop_on_sync: false,
            no_pending_block: false,
            service_state_sender: Default::default(),
        }
    }
}

pub struct SyncController<P: ForwardPipeline> {
    forward_pipeline: P,
    config: SyncControllerConfig,
    current_l1_head: Option<StateUpdate>,
    probe: ThrottledRepeatedFuture<ProviderBlockHeader>,
    sync_metrics: SyncMetrics,
    status: Option<ServiceEvent>,
    get_pending_block: Option<ThrottledRepeatedFuture<()>>,
    backend: Arc<MadaraBackend>,
}

impl<P: ForwardPipeline> SyncController<P> {
    pub fn set_status(&mut self, status: ServiceEvent) {
        if self.status != Some(status) {
            self.config.service_state_sender.send(status);
            self.status = Some(status);
        }
    }

    pub fn new(
        backend: Arc<MadaraBackend>,
        forward_pipeline: P,
        probe: ThrottledRepeatedFuture<ProviderBlockHeader>,
        config: SyncControllerConfig,
        get_pending_block: Option<ThrottledRepeatedFuture<()>>,
    ) -> Self {
        Self {
            sync_metrics: SyncMetrics::register(forward_pipeline.next_input_block_n()),
            get_pending_block: get_pending_block.filter(|_| !config.no_pending_block),
            forward_pipeline,
            config,
            current_l1_head: None,
            probe,
            status: None,
            backend,
        }
    }

    pub async fn run(&mut self, mut ctx: mp_utils::service::ServiceContext) -> anyhow::Result<()> {

        // Get the pre-sync status
        let first_block = self.backend.get_latest_applied_trie_update()?.unwrap_or(0);
        println!("First block: {}", first_block);

        let interval_duration = Duration::from_secs(3);
        let mut interval = tokio::time::interval_at(Instant::now() + interval_duration, interval_duration);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        self.set_status(ServiceEvent::Starting);
        loop {
            tokio::select! {
                _ = ctx.cancelled() => return Ok(()),
                _ = interval.tick() => self.show_status(),
                res = self.run_inner() => break res?,
            }
        }
        self.show_status();

        // TODO: will this also depend on the Starknet version?
        // TODO: add feature flag to enable/disable this functionality
        // TODO: add appropriate CLI flags

        // Define the block range for state root calculation
        let latest_block = match self.backend.db.get_chain_tip()? {
            StorageChainTip::Confirmed(block_number) => block_number,
            _ => return Err(anyhow!("Chain tip is not confirmed")),
        };

        println!("SNAP-SYNC: Processing blocks {} to {}", first_block, latest_block);

        // Collect all state diffs first WITHOUT any pre_range checks

        let mut state_diff_map = StateDiffMap::default();

        for block_number in first_block..=latest_block {
            let view = self.backend.block_view(BlockId::Number(block_number))?;
            let single_contract_state_diff = view.get_state_diff()?;
            state_diff_map.apply_state_diff(&single_contract_state_diff);
        }

        let state_diff = {
            let mut state_diff = state_diff_map.to_raw_state_diff();
            state_diff.sort();
            state_diff
        };

        let pre_range_block_check = if first_block == 0 {
            None
        } else {
            Some(first_block.saturating_sub(1))
        };

        let accumulated_state_diff = compress_state_diff(
            state_diff,
            pre_range_block_check,
            self.backend.clone()
        ).await?;

        println!("SNAP-SYNC: Raw squash complete. Now compressing with pre_range_block={}...", first_block - 1);

        let global_state_root = self.backend
            .write_access()
            .apply_to_global_trie(first_block, vec![accumulated_state_diff].iter())?;

        self.backend.write_latest_applied_trie_update(&latest_block.checked_sub(1))?;

        println!("Global state root: {:?}", global_state_root);

        // Handle shutdown based on configuration
        if self.config.global_stop_on_sync {
            tracing::info!("🌐 Reached stop-on-sync condition, shutting down node...");
            ctx.cancel_global();
        } else {
            tracing::info!("🌐 Sync process ended");
        }

        Ok(())
    }

    fn target_height(&self) -> Option<u64> {
        let mut target_block = cmp::max(
            self.current_l1_head.as_ref().and_then(|h| h.block_number),
            self.probe.last_val().map(|v| v.block_number),
        );

        // Bound by stop_at_block_n
        if let Some(stop_at) = self.config.stop_at_block_n {
            if target_block >= Some(stop_at) {
                target_block = Some(stop_at)
            }
        }

        target_block
    }

    async fn run_inner(&mut self) -> anyhow::Result<()> {
        loop {
            let target_height = self.target_height();

            let can_run_pipeline = !self.forward_pipeline.is_empty()
                || target_height.is_some_and(|b| b >= self.forward_pipeline.next_input_block_n());
            tracing::trace!(
                "can run {:?} {:?} {}",
                can_run_pipeline,
                target_height,
                self.forward_pipeline.next_input_block_n()
            );

            let probe_height = if let Some(v) = self.probe.last_val() {
                self.backend.set_sync_status(SyncStatus::Running {
                    highest_block_n: v.block_number,
                    highest_block_hash: v.block_hash,
                });
                Some(v.block_number)
            } else {
                None
            };

            let target = target_height.filter(|_| can_run_pipeline);

            if let Some(target) = target {
                self.set_status(ServiceEvent::SyncingTo { target });
            } else {
                self.set_status(ServiceEvent::Idle);
            }

            if self.forward_pipeline.is_empty()
                && self
                    .config
                    .stop_at_block_n
                    .is_some_and(|stop_at| self.forward_pipeline.next_input_block_n() > stop_at)
                && !self.pending_block_task_is_running()
            {
                // End condition for stop_at_block_n.
                tracing::debug!("End condition for stop_at");
                break Ok(());
            }

            tokio::select! {
                Ok(()) = self.config.l1_head_recv.changed() => {
                    self.current_l1_head = self.config.l1_head_recv.borrow_and_update().clone();
                }
                Some(res) = OptionFuture::from(
                    target.map(|target| self.forward_pipeline.run(target, probe_height, &mut self.sync_metrics))
                ) => {
                    res?;
                }
                res = self.probe.run() => {
                    let new_probe_height = res?.map(|v| v.block_number);
                    if self.config.stop_at_block_n.is_none()
                        && !can_run_pipeline
                        && self.config.stop_on_sync
                        && probe_height == new_probe_height
                        && !self.pending_block_task_is_running()
                    {
                        // The Probe returned the same thing as last time, and we cannot run the pipeline.
                        // This is the exit condition when stop_on_sync is enabled,
                        // except if there is a stop_at_block_n.
                        break Ok(());
                    }
                }
                // We only run the pending block task if there is no more work to be done in the inner pipeline.
                Some(res) = OptionFuture::from(
                    self.get_pending_block.as_mut().filter(|_| !can_run_pipeline).map(|fut| fut.run())
                ) => {
                    let res = res?;
                    tracing::debug!("Pending probe successful: {}", res.is_some());
                    if res.is_some() {
                        self.config.service_state_sender.send(ServiceEvent::UpdatedPreconfirmedBlock);
                    }
                }
                else => break Ok(()),
            }
        }
    }

    fn pending_block_task_is_running(&self) -> bool {
        self.get_pending_block.as_ref().is_some_and(|p| p.is_running())
    }

    fn show_status(&self) {
        use crate::util::fmt_option;

        let latest_block = self.forward_pipeline.latest_block();
        let throughput_sec = self.sync_metrics.counter.get_throughput();
        let target_height = self.target_height();
        self.forward_pipeline.show_status();

        // fmt_option will unwrap the Option or else show the given string

        tracing::info!(
            "🔗 Sync is at {}/{} [{throughput_sec:.2} blocks/s]",
            fmt_option(latest_block, "N"),
            fmt_option(target_height, "?")
        );
    }
}
