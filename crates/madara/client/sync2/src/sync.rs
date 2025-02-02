use crate::counter::ThroughputCounter;
use futures::{
    future::{BoxFuture, OptionFuture},
    Future, FutureExt,
};
use mc_eth::state_update::{L1HeadReceiver, L1StateUpdate};
use std::{sync::Arc, time::Duration};
use tokio::time::Instant;

pub trait ForwardPipeline {
    fn run(&mut self, target_block_n: u64) -> impl Future<Output = anyhow::Result<()>> + Send;
    fn next_input_block_n(&self) -> u64;
    fn show_status(&self);
    fn throughput_counter(&self) -> &ThroughputCounter;
    /// Return false when no work can be done.
    fn is_empty(&self) -> bool;
    fn latest_block(&self) -> Option<u64>;
}

pub trait Probe {
    /// Returns the new highest known block.
    fn forward_probe(
        self: Arc<Self>,
        next_block_n: u64,
    ) -> impl Future<Output = anyhow::Result<Option<u64>>> + Send + 'static;
}

pub struct SyncControllerConfig {
    pub l1_head_recv: L1HeadReceiver,
    pub stop_at_block_n: Option<u64>,
    pub stop_on_sync: bool,
}

pub struct SyncController<P: ForwardPipeline, R: Probe> {
    forward_pipeline: P,
    probe: Option<Arc<R>>,
    config: SyncControllerConfig,
    current_l1_head: Option<L1StateUpdate>,
    current_probe_future: Option<BoxFuture<'static, anyhow::Result<Option<u64>>>>,
    probe_highest_known_block: Option<u64>,
    probe_wait_deadline: Option<Instant>,
}

/// Avoid spamming the probe.
const PROBE_WAIT_DELAY: Duration = Duration::from_secs(2);
impl<P: ForwardPipeline, R: Probe> SyncController<P, R> {
    pub fn new(forward_pipeline: P, probe: Option<Arc<R>>, config: SyncControllerConfig) -> Self {
        Self {
            forward_pipeline,
            probe,
            config,
            current_l1_head: None,
            current_probe_future: None,
            probe_highest_known_block: Default::default(),
            probe_wait_deadline: None,
        }
    }

    pub async fn run(&mut self, mut ctx: mp_utils::service::ServiceContext) -> anyhow::Result<()> {
        let mut interval = tokio::time::interval(Duration::from_secs(3));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = ctx.cancelled() => break Ok(()),
                _ = interval.tick() => self.show_status(),
                res = self.run_inner() => {
                    res?;
                    self.show_status();
                    if self.config.stop_on_sync {
                        tracing::info!("🌐 Reached stop-on-sync condition, shutting down node...");
                        ctx.cancel_global();
                    } else {
                        tracing::info!("🌐 Sync process ended");
                    }
                    break Ok(())
                }
            }
        }
    }

    fn target_height(&self) -> Option<u64> {
        fn aggregate_options(a: Option<u64>, b: Option<u64>, f: impl FnOnce(u64, u64) -> u64) -> Option<u64> {
            match (a, b) {
                (None, None) => None,
                (None, Some(b)) => Some(b),
                (Some(a), None) => Some(a),
                (Some(a), Some(b)) => Some(f(a, b)),
            }
        }

        let mut target_block = self.current_l1_head.as_ref().map(|h| h.block_number);
        target_block = aggregate_options(target_block, self.probe_highest_known_block, u64::max);

        // Bound by stop_at_block_n

        aggregate_options(target_block, self.config.stop_at_block_n, u64::min)
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

            if let Some(probe) = &self.probe {
                tracing::trace!("run inner {:?} {:?}", self.forward_pipeline.next_input_block_n(), target_height);
                if self.current_probe_future.is_none() && !can_run_pipeline {
                    let fut = probe.clone().forward_probe(self.forward_pipeline.next_input_block_n());
                    let delay = self.probe_wait_deadline;

                    self.current_probe_future = Some(
                        async move {
                            if let Some(deadline) = delay {
                                tokio::time::sleep_until(deadline).await;
                            }
                            fut.await
                        }
                        .boxed(),
                    );
                }
            }

            tokio::select! {
                Ok(()) = self.config.l1_head_recv.changed() => {
                    self.current_l1_head = self.config.l1_head_recv.borrow_and_update().clone();
                }
                Some(res) = OptionFuture::from(self.current_probe_future.as_mut()) => {
                    self.current_probe_future = None;
                    self.probe_wait_deadline = None;
                    let probe_new_highest_block = res?;
                    // Only delay the probe when it did not return any new block.
                    if self.probe_highest_known_block == probe_new_highest_block {
                        self.probe_wait_deadline = Some(Instant::now() + PROBE_WAIT_DELAY);
                    }
                    self.probe_highest_known_block = probe_new_highest_block;
                    tracing::trace!("probe result {:?}", self.probe_highest_known_block);
                }
                Some(res) = OptionFuture::from(
                    target_height.filter(|_| can_run_pipeline)
                        .map(|target| self.forward_pipeline.run(target))
                ) =>
                {
                    res?;
                }
                else => break Ok(()),
            }
        }
    }

    fn show_status(&self) {
        use crate::util::fmt_option;

        let latest_block = self.forward_pipeline.latest_block();
        let throughput_sec = self.forward_pipeline.throughput_counter().get_throughput();
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
