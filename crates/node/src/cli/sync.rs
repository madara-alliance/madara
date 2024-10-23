use std::{sync::Arc, time::Duration};

use mp_chain_config::ChainConfig;
use starknet_api::core::ChainId;

use mc_sync::fetch::fetchers::FetchConfig;
use mp_utils::parsers::{parse_duration, parse_url};
use url::Url;

#[derive(Clone, Debug, clap::Args)]
pub struct SyncParams {
    /// Disable the sync service. The sync service is responsible for listening for new blocks on starknet and ethereum.
    #[clap(env = "MADARA_SYNC_DISABLED", long, alias = "no-sync")]
    pub sync_disabled: bool,

    /// The block you want to start syncing from. This will most probably break your database.
    #[clap(env = "MADARA_UNSAFE_STARTING_BLOCK", long, value_name = "BLOCK NUMBER")]
    pub unsafe_starting_block: Option<u64>,

    /// Disable state root verification. When importing a block, the state root verification is the most expensive operation.
    /// Disabling it will mean the sync service will have a huge speed-up, at a security cost
    // TODO(docs): explain the security cost
    #[clap(env = "MADARA_DISABLE_ROOT", long)]
    pub disable_root: bool,

    /// Gateway api key to avoid rate limiting (optional).
    #[clap(env = "MADARA_GATEWAY_KEY", long, value_name = "API KEY")]
    pub gateway_key: Option<String>,

    /// Feeder gateway url used to sync blocks, state updates and classes
    #[clap(env = "MADARA_GATEWAY_URL", long, value_parser = parse_url, value_name = "URL")]
    pub gateway_url: Option<Url>,

    /// Polling interval, in seconds. This only affects the sync service once it has caught up with the blockchain tip.
    #[clap(
		env = "MADARA_SYNC_POLLING_INTERVAL",
        long,
        value_parser = parse_duration,
        default_value = "4s",
        value_name = "SYNC POLLING INTERVAL",
        help = "Set the sync polling interval (e.g., '4s', '100ms', '1min')"
    )]
    pub sync_polling_interval: Duration,

    /// Pending block polling interval, in seconds. This only affects the sync service once it has caught up with the blockchain tip.
    #[clap(
		env = "MADARA_PENDING_BLOCK_POLL_INTERVAL",
        long,
        value_parser = parse_duration,
        default_value = "2s",
        value_name = "PENDING BLOCK POLL INTERVAL",
        help = "Set the pending block poll interval (e.g., '2s', '500ms', '30s')"
    )]
    pub pending_block_poll_interval: Duration,

    /// Disable sync polling. This currently means that the sync process will not import any more block once it has caught up with the
    /// blockchain tip.
    #[clap(env = "MADARA_NO_SYNC_POLLING", long)]
    pub no_sync_polling: bool,

    /// Number of blocks to sync. May be useful for benchmarking the sync service.
    #[clap(env = "MADARA_N_BLOCKS_TO_SYNC", long, value_name = "NUMBER OF BLOCKS")]
    pub n_blocks_to_sync: Option<u64>,

    /// Periodically create a backup, for debugging purposes. Use it with `--backup-dir <PATH>`.
    #[clap(env = "MADARA_BACKUP_EVERY_N_BLOCKS", long, value_name = "NUMBER OF BLOCKS")]
    pub backup_every_n_blocks: Option<u64>,
}

impl SyncParams {
    pub fn block_fetch_config(&self, chain_id: ChainId, chain_config: Arc<ChainConfig>) -> FetchConfig {
        let (gateway, feeder_gateway) = match &self.gateway_url {
            Some(url) => (
                url.join("/gateway/").expect("Error parsing url"),
                url.join("/feeder_gateway/").expect("Error parsing url"),
            ),
            None => (chain_config.gateway_url.clone(), chain_config.feeder_gateway_url.clone()),
        };

        let polling = if self.no_sync_polling { None } else { Some(self.sync_polling_interval) };

        FetchConfig {
            gateway,
            feeder_gateway,
            chain_id,
            verify: !self.disable_root,
            api_key: self.gateway_key.clone(),
            sync_polling_interval: polling,
            n_blocks_to_sync: self.n_blocks_to_sync,
        }
    }
}
