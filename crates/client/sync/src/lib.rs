#![allow(deprecated)]
#![feature(let_chains)]

// use std::sync::Arc;
// use sp_runtime::traits::Block as BlockT;
// use reqwest::Url;

pub mod commitments;
pub mod fetch;
pub mod l1;
pub mod l2;
pub mod reorgs;
pub mod utils;
pub mod metrics;

pub use l2::SenderConfig;
pub use mp_types::block::{DBlockT, DHashT};
#[cfg(feature = "m")]
pub use utils::m;
pub use utils::{convert, utility};

use crate::l2::L2SyncConfig;

type CommandSink = futures::channel::mpsc::Sender<sc_consensus_manual_seal::rpc::EngineCommand<sp_core::H256>>;

pub mod starknet_sync_worker {
    use std::sync::Arc;

    use anyhow::Context;
    use mp_block::DeoxysBlock;
    use mp_convert::state_update::ToStateUpdateCore;
    use prometheus_endpoint::prometheus;
    use reqwest::Url;
    use sp_blockchain::HeaderBackend;
    use starknet_providers::sequencer::models::BlockId;
    use starknet_providers::SequencerGatewayProvider;
    use tokio::sync::mpsc::Sender;

    use self::fetch::fetchers::FetchConfig;
    use super::*;
    use crate::l2::verify_l2;
    use crate::metrics::block_metrics::BlockMetrics;

    pub async fn sync<C>(
        fetch_config: FetchConfig,
        block_sender: Sender<DeoxysBlock>,
        command_sink: CommandSink,
        l1_url: Url,
        client: Arc<C>,
        starting_block: u32,
        backup_every_n_blocks: Option<usize>,
        prometheus_registry: Option<prometheus::Registry>,
    ) -> anyhow::Result<()>
    where
        C: HeaderBackend<DBlockT> + 'static,
    {
        let block_metrics =
            prometheus_registry.and_then(|registry| BlockMetrics::register(&registry).ok());

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
            res = l1::sync(l1_url.clone(), block_metrics) => res.context("syncing L1 state")?,
            res = l2::sync(
                block_sender,
                command_sink,
                provider,
                client,
                L2SyncConfig {
                    first_block: starting_block.into(),
                    n_blocks_to_sync: fetch_config.n_blocks_to_sync,
                    verify: fetch_config.verify,
                    sync_polling_interval: fetch_config.sync_polling_interval,
                    backup_every_n_blocks,
                },
            ) => res.context("syncing L2 state")?
        );

        Ok(())
    }
}
