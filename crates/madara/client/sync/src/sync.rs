use crate::{metrics::SyncMetrics, probe::ProbeState};
use futures::{future::OptionFuture, Future};
use mc_eth::state_update::{L1HeadReceiver, L1StateUpdate};
use tokio::time::Instant;
use std::{cmp, time::Duration};

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

pub struct SyncControllerConfig {
    pub l1_head_recv: L1HeadReceiver,
    pub stop_at_block_n: Option<u64>,
    pub global_stop_on_sync: bool,
    pub stop_on_sync: bool,
}

pub struct SyncController<P: ForwardPipeline> {
    forward_pipeline: P,
    config: SyncControllerConfig,
    current_l1_head: Option<L1StateUpdate>,
    probe: ProbeState<u64>,
    sync_metrics: SyncMetrics,
}

impl<P: ForwardPipeline> SyncController<P> {
    pub fn new(forward_pipeline: P, probe: ProbeState<u64>, config: SyncControllerConfig) -> Self {
        Self {
            sync_metrics: SyncMetrics::register(forward_pipeline.next_input_block_n()),
            forward_pipeline,
            config,
            current_l1_head: None,
            probe,
        }
    }

    pub async fn run(&mut self, mut ctx: mp_utils::service::ServiceContext) -> anyhow::Result<()> {
        let interval_duration = Duration::from_secs(3);
        let mut interval = tokio::time::interval_at(Instant::now() + interval_duration, interval_duration);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
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
        let mut target_block = self.current_l1_head.as_ref().map(|h| h.block_number);
        target_block = cmp::max(target_block, self.probe.last_val());

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
            if self.forward_pipeline.is_empty()
                && self
                    .config
                    .stop_at_block_n
                    .is_some_and(|stop_at| self.forward_pipeline.next_input_block_n() > stop_at)
            {
                // End condition
                break Ok(());
            }
            let target_height = self.target_height();

            let can_run_pipeline = !self.forward_pipeline.is_empty()
                || target_height.is_some_and(|b| b >= self.forward_pipeline.next_input_block_n());
            tracing::trace!(
                "can run {:?} {:?} {}",
                can_run_pipeline,
                target_height,
                self.forward_pipeline.next_input_block_n()
            );

            let probe_height = self.probe.last_val();

            tokio::select! {
                Ok(()) = self.config.l1_head_recv.changed() => {
                    self.current_l1_head = self.config.l1_head_recv.borrow_and_update().clone();
                }
                Some(res) = OptionFuture::from(
                    target_height.filter(|_| can_run_pipeline)
                        .map(|target| self.forward_pipeline.run(target, probe_height, &mut self.sync_metrics))
                ) =>
                {
                    res?;
                }
                res = self.probe.run() => {
                    if probe_height == res? && !can_run_pipeline && self.config.stop_on_sync {
                        // Probe returned the same thing as last time, and we cannot run the pipeline.
                        break Ok(())
                    }
                }
                else => break Ok(()),
            }
        }
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
