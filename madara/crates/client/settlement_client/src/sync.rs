use crate::gas_price::L1BlockMetrics;
use crate::messaging::sync;
use crate::state_update::{state_update_worker, L1HeadSender};
use crate::L1ClientImpl;
use mc_mempool::GasPriceProvider;
use mp_utils::service::ServiceContext;
use std::sync::Arc;
use std::time::Duration;

pub struct SyncWorkerConfig {
    pub l1_gas_provider: GasPriceProvider,
    pub gas_price_sync_disabled: bool,
    pub gas_price_poll: Duration,
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
        ));

        if !config.gas_price_sync_disabled {
            let client_ = self.clone();
            join_set.spawn(async move {
                client_
                    .gas_price_worker(
                        config.l1_gas_provider,
                        config.gas_price_poll,
                        ctx.clone(),
                        config.l1_block_metrics,
                    )
                    .await
            });
        }

        while let Some(res) = join_set.join_next().await {
            res??;
        }

        Ok(())
    }
}
