use std::sync::Arc;
use std::time::Duration;

use crate::gas_price::{gas_price_worker, GasPriceProviderConfig, L1BlockMetrics};
use crate::messaging::sync;
use crate::state_update::{state_update_worker, L1HeadSender};
use crate::L1ClientImpl;

use mp_utils::service::ServiceContext;

pub struct SyncWorkerConfig {
    pub gas_provider_config: GasPriceProviderConfig,
    pub l1_msg_min_confirmations: u64,
    pub block_poll_interval: Duration,
    pub l1_block_metrics: Arc<L1BlockMetrics>,
    pub l1_head_sender: L1HeadSender,
}

impl L1ClientImpl {
    pub async fn run_sync_worker(self: Arc<Self>, ctx: ServiceContext, config: SyncWorkerConfig) -> anyhow::Result<()> {
        let mut join_set = tokio::task::JoinSet::new();

        join_set.spawn(state_update_worker(
            Arc::clone(&self.backend),
            self.provider.clone(),
            ctx.clone(),
            config.l1_head_sender,
            config.l1_block_metrics.clone(),
        ));

        join_set.spawn(sync(
            self.provider.clone(),
            Arc::clone(&self.backend),
            self.notify_new_message_to_l2.clone(),
            ctx.clone(),
            config.l1_msg_min_confirmations,
            config.block_poll_interval,
        ));

        if !config.gas_provider_config.all_is_fixed() {
            join_set.spawn(gas_price_worker(
                self.provider.clone(),
                Arc::clone(&self.backend),
                ctx.clone(),
                config.gas_provider_config,
                config.l1_block_metrics,
            ));
        }

        while let Some(res) = join_set.join_next().await {
            res??;
        }

        Ok(())
    }
}
