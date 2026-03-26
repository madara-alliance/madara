use std::sync::Arc;

use crate::gas_price::{gas_price_worker, GasPriceProviderConfig, L1BlockMetrics};
use crate::messaging::sync;
use crate::state_update::{state_update_worker, L1HeadSender};
use crate::L1ClientImpl;

use mp_utils::service::ServiceContext;

pub struct SyncWorkerConfig {
    pub gas_provider_config: GasPriceProviderConfig,
    pub l1_block_metrics: Arc<L1BlockMetrics>,
    pub l1_head_sender: L1HeadSender,
    pub unsafe_skip_l1_message_consumed_check: bool,
    pub unsafe_no_l1_handler_tx_creation_from_message: bool,
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

        // TODO(18/02/2026,@prkpndy): we do not need message syncing service when not doing block production
        // keeping it for now because when converting a full node (with L1 sync enabled) to
        // sequencer, we cannot start just the message syncing service (we don't have the capability
        // to toggle services inside a service)
        join_set.spawn(sync(
            self.provider.clone(),
            Arc::clone(&self.backend),
            self.notify_new_message_to_l2.clone(),
            ctx.clone(),
            config.unsafe_skip_l1_message_consumed_check,
            config.unsafe_no_l1_handler_tx_creation_from_message,
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
