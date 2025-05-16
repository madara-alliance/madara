use crate::{metrics::SyncMetrics, probe::ThrottledRepeatedFuture, util::ServiceStateSender};
use futures::{future::OptionFuture, Future};
use mc_db::{MadaraBackend, SyncStatus};
use mc_settlement_client::state_update::{L1HeadReceiver, StateUpdate};
use mp_gateway::block::ProviderBlockHeader;
use std::sync::Arc;
use std::{cmp, time::Duration};
use tokio::time::Instant;

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
    UpdatedPendingBlock,
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
    /// Stop the service once fully synced, meaning the pipeline cannot be run again and the probe did not return
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
        if self.config.global_stop_on_sync {
            tracing::info!("🌐 Reached stop-on-sync condition, shutting down node...");
            ctx.cancel_global();
        } else {
            tracing::info!("🌐 Sync process ended");
        }
        Ok(())
    }

    fn target_height(&self) -> Option<u64> {
        let current_head = self.current_l1_head.as_ref().and_then(|h| h.block_number);
        let probe_block = self.probe.last_val().map(|v| v.block_number);

        let target_block = match (current_head, probe_block) {
            (Some(head), Some(probe)) => Some(cmp::max(head, probe)),
            (Some(head), None) => Some(head),
            (None, Some(probe)) => Some(probe),
            (None, None) => None,
        };

        // Bound by stop_at_block_n
        match (target_block, self.config.stop_at_block_n) {
            (Some(target), Some(stop_at)) if target >= stop_at => Some(stop_at),
            _ => target_block,
        }
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
                self.backend
                    .set_sync_status(SyncStatus::Running {
                        highest_block_n: v.block_number,
                        highest_block_hash: v.block_hash,
                    })
                    .await;
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
                        // Probe returned the same thing as last time, and we cannot run the pipeline.
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
                        self.config.service_state_sender.send(ServiceEvent::UpdatedPendingBlock);
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
