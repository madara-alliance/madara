use crate::cli::NetworkType;
use mc_sync::fetch::fetchers::FetchConfig;
use starknet_api::core::ChainId;
use std::time::Duration;

#[derive(Clone, Debug, clap::Args)]
pub struct SyncParams {
    /// Disable the sync service. The sync service is responsible for listening for new blocks on starknet and ethereum.
    #[clap(long, alias = "no-sync")]
    pub sync_disabled: bool,

    /// The block you want to start syncing from. This will most probably break your database.
    #[clap(long, value_name = "BLOCK NUMBER")]
    pub unsafe_starting_block: Option<u64>,

    /// This will produce sound interpreted from the block hashes.
    #[cfg(feature = "sound")]
    #[clap(long)]
    pub sound: bool,

    /// Disable state root verification. When importing a block, the state root verification is the most expensive operation.
    /// Disabling it will mean the sync service will have a huge speed-up, at a security cost
    // TODO(docs): explain the security cost
    #[clap(long)]
    pub disable_root: bool,

    /// Gateway api key to avoid rate limiting (optional).
    #[clap(long, value_name = "API KEY")]
    pub gateway_key: Option<String>,

    /// Polling interval, in seconds. This only affects the sync service once it has caught up with the blockchain tip.
    #[clap(long, default_value = "4", value_name = "SECONDS")]
    pub sync_polling_interval: u64,

    /// Pending block polling interval, in seconds. This only affects the sync service once it has caught up with the blockchain tip.
    #[clap(long, default_value = "2", value_name = "SECONDS")]
    pub pending_block_poll_interval: u64,

    /// Disable sync polling. This currently means that the sync process will not import any more block once it has caught up with the
    /// blockchain tip.
    #[clap(long)]
    pub no_sync_polling: bool,

    /// Number of blocks to sync. May be useful for benchmarking the sync service.
    #[clap(long, value_name = "NUMBER OF BLOCKS")]
    pub n_blocks_to_sync: Option<u64>,

    /// Periodically create a backup, for debugging purposes. Use it with `--backup-dir <PATH>`.
    #[clap(long, value_name = "NUMBER OF BLOCKS")]
    pub backup_every_n_blocks: Option<u64>,
}

impl SyncParams {
    pub fn block_fetch_config(&self, chain_id: ChainId, network: NetworkType) -> FetchConfig {
        let gateway = network.gateway();
        let feeder_gateway = network.feeder_gateway();

        let polling = if self.no_sync_polling { None } else { Some(Duration::from_secs(self.sync_polling_interval)) };

        #[cfg(feature = "sound")]
        let sound = self.sound;
        #[cfg(not(feature = "sound"))]
        let sound = false;

        FetchConfig {
            gateway,
            feeder_gateway,
            chain_id,
            sound,
            verify: !self.disable_root,
            api_key: self.gateway_key.clone(),
            sync_polling_interval: polling,
            n_blocks_to_sync: self.n_blocks_to_sync,
        }
    }
}
