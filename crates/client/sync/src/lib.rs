#![allow(deprecated)]

pub mod commitments;
pub mod fetch;
pub mod l1;
pub mod l2;
pub mod metrics;
pub mod reorgs;
pub mod utils;

pub use eth;

#[cfg(feature = "m")]
pub use utils::m;
pub use utils::{convert, utility};

use crate::l2::L2SyncConfig;

use starknet_types_core::felt::Felt;

pub mod starknet_sync_worker {
    use std::{sync::Arc, time::Duration};

    use anyhow::Context;
    use dc_db::{db_metrics::DbMetrics, DeoxysBackend};
    use dc_telemetry::TelemetryHandle;
    use reqwest::Url;
    use starknet_providers::SequencerGatewayProvider;

    use self::fetch::fetchers::FetchConfig;
    use super::*;
    use crate::metrics::block_metrics::BlockMetrics;

    #[allow(clippy::too_many_arguments)]
    pub async fn sync(
        backend: &Arc<DeoxysBackend>,
        fetch_config: FetchConfig,
        l1_url: Option<Url>,
        l1_core_address: ethers::abi::Address,
        starting_block: Option<u64>,
        backup_every_n_blocks: Option<u64>,
        block_metrics: BlockMetrics,
        db_metrics: DbMetrics,
        chain_id: Felt,
        telemetry: TelemetryHandle,
        pending_block_poll_interval: Duration,
    ) -> anyhow::Result<()> {
        // let starting_block = starting_block + 1;

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

        let provider = SequencerGatewayProvider::new(
            fetch_config.gateway.clone(),
            fetch_config.feeder_gateway.clone(),
            fetch_config.chain_id,
        );
        let provider = match &fetch_config.api_key {
            Some(api_key) => provider.with_header("X-Throttling-Bypass".to_string(), api_key.clone()),
            None => provider,
        };

        let l1_block_metric = block_metrics.clone();
        let l1_fut = async {
            if let Some(l1_url) = l1_url {
                l1::sync(backend, l1_url.clone(), l1_block_metric, l1_core_address, chain_id).await
            } else {
                Ok(())
            }
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
