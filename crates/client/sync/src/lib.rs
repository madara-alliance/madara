use crate::l2::L2SyncConfig;

pub mod commitments;
pub mod fetch;
pub mod l2;
pub mod metrics;
pub mod reorgs;
pub mod utils;

#[cfg(feature = "m")]
pub use utils::m;
pub use utils::{convert, utility};

pub mod starknet_sync_worker {
    use super::*;
    use crate::metrics::block_metrics::BlockMetrics;
    use anyhow::Context;
    use dc_db::{db_metrics::DbMetrics, DeoxysBackend};
    use dc_eth::client::EthereumClient;
    use dc_telemetry::TelemetryHandle;
    use dp_convert::ToFelt;
    use fetch::fetchers::FetchConfig;
    use starknet_providers::SequencerGatewayProvider;
    use std::{sync::Arc, time::Duration};

    #[allow(clippy::too_many_arguments)]
    pub async fn sync(
        backend: &Arc<DeoxysBackend>,
        fetch_config: FetchConfig,
        eth_client: Option<EthereumClient>,
        starting_block: Option<u64>,
        backup_every_n_blocks: Option<u64>,
        block_metrics: BlockMetrics,
        db_metrics: DbMetrics,
        telemetry: TelemetryHandle,
        pending_block_poll_interval: Duration,
    ) -> anyhow::Result<()> {
        let starting_block = if let Some(starting_block) = starting_block {
            starting_block
        } else {
            backend
                .get_block_n(&dp_block::BlockId::Tag(dp_block::BlockTag::Latest))
                .context("getting sync tip")?
                .map(|block_id| block_id + 1) // next block after the tip
                .unwrap_or_default() as _ // or genesis
        };

        log::info!("⛓️  Starting L2 sync from block {}", starting_block);

        let chain_id = fetch_config.chain_id.to_felt();
        let provider =
            SequencerGatewayProvider::new(fetch_config.gateway.clone(), fetch_config.feeder_gateway.clone(), chain_id);
        let provider = match &fetch_config.api_key {
            Some(api_key) => provider.with_header("X-Throttling-Bypass".to_string(), api_key.clone()),
            None => provider,
        };

        let l1_fut = async move {
            if let Some(eth_client) = eth_client {
                dc_eth::state_update::sync(backend, &eth_client, chain_id).await?;
            }
            anyhow::Ok(())
        };

        tokio::try_join!(
            l1_fut,
            l2::sync(
                backend,
                provider,
                L2SyncConfig {
                    first_block: starting_block,
                    n_blocks_to_sync: fetch_config.n_blocks_to_sync,
                    verify: fetch_config.verify,
                    sync_polling_interval: fetch_config.sync_polling_interval,
                    backup_every_n_blocks,
                    pending_block_poll_interval,
                },
                block_metrics,
                db_metrics,
                starting_block,
                chain_id,
                telemetry,
            ),
        )?;

        Ok(())
    }
}
