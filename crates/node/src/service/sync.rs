use mc_sync::{fetch::fetchers::FetchConfig, metrics::block_metrics::BlockMetrics};
use sp_core::H160;
use starknet_core::types::FieldElement;
use tokio::task::JoinSet;
use url::Url;

use crate::cli::SyncParams;

#[derive(Clone)]
pub struct SyncService {
    fetch_config: FetchConfig,
    backup_every_n_blocks: Option<u64>,
    l1_endpoint: Url,
    l1_core_address: H160,
    starting_block: Option<u64>,
    block_metrics: Option<BlockMetrics>,
    chain_id: FieldElement,
}

impl SyncService {
    pub fn new(config: &SyncParams, block_metrics: Option<BlockMetrics>) -> anyhow::Result<Self> {
        Ok(Self {
            fetch_config: config.block_fetch_config(),
            l1_endpoint: config.l1_endpoint.clone(),
            l1_core_address: config.network.l1_core_address(),
            starting_block: config.starting_block,
            backup_every_n_blocks: config.backup_every_n_blocks,
            block_metrics,
            chain_id: config.network.chain_id(),
        })
    }
    pub async fn start(&mut self, join_set: &mut JoinSet<anyhow::Result<()>>) -> anyhow::Result<()> {
        let SyncService {
            fetch_config,
            backup_every_n_blocks,
            l1_endpoint,
            l1_core_address,
            starting_block,
            block_metrics,
            chain_id,
        } = self.clone();

        join_set.spawn(async move {
            mc_sync::starknet_sync_worker::sync(
                fetch_config,
                l1_endpoint,
                l1_core_address,
                starting_block,
                backup_every_n_blocks,
                block_metrics,
                chain_id,
            )
            .await
        });

        Ok(())
    }
}
