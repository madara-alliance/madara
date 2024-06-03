#![allow(deprecated)]
#![feature(let_chains)]

pub mod commitments;
pub mod fetch;
pub mod l1;
pub mod l2;
pub mod metrics;
pub mod reorgs;
pub mod utils;

pub use mp_types::block::{DBlockT, DHashT};
#[cfg(feature = "m")]
pub use utils::m;
pub use utils::{convert, utility};

use crate::l2::L2SyncConfig;

pub mod starknet_sync_worker {
    use anyhow::Context;
    use mp_convert::state_update::ToStateUpdateCore;
    use reqwest::Url;
    use starknet_providers::sequencer::models::BlockId;
    use starknet_providers::SequencerGatewayProvider;

    use self::fetch::fetchers::FetchConfig;
    use super::*;
    use crate::l2::verify_l2;
    use crate::metrics::block_metrics::BlockMetrics;

    pub async fn sync(
        fetch_config: FetchConfig,
        l1_url: Url,
        starting_block: u32,
        backup_every_n_blocks: Option<usize>,
        block_metrics: Option<BlockMetrics>,
    ) -> anyhow::Result<()> {
        let starting_block = starting_block + 1;

        let provider = SequencerGatewayProvider::new(
            fetch_config.gateway.clone(),
            fetch_config.feeder_gateway.clone(),
            fetch_config.chain_id,
        );
        let provider = match &fetch_config.api_key {
            Some(api_key) => provider.with_header("X-Throttling-Bypass".to_string(), api_key.clone()),
            None => provider,
        };

        if starting_block == 1 {
            let state_update = provider
                .get_state_update(BlockId::Number(0))
                .await
                .context("getting state update for genesis block")?
                .to_state_update_core();
            verify_l2(0, &state_update)?;
        }

        tokio::select!(
            res = l1::sync(l1_url.clone(), block_metrics.clone()) => res.context("syncing L1 state")?,
            res = l2::sync(
                provider,
                L2SyncConfig {
                    first_block: starting_block.into(),
                    n_blocks_to_sync: fetch_config.n_blocks_to_sync,
                    verify: fetch_config.verify,
                    sync_polling_interval: fetch_config.sync_polling_interval,
                    backup_every_n_blocks,
                },
                block_metrics,
            ) => res.context("syncing L2 state")?
        );

        Ok(())
    }
}
