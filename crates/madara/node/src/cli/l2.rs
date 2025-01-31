use anyhow::Context;
use http::HeaderName;
use http::HeaderValue;
use mc_gateway_client::GatewayProvider;
use mp_chain_config::ChainConfig;
use mp_utils::parsers::parse_url;
use std::sync::Arc;
use url::Url;

use super::FGW_DEFAULT_PORT;
use super::RPC_DEFAULT_PORT_ADMIN;

#[derive(Clone, Debug, clap::Args)]
pub struct L2SyncParams {
    /// Disable the sync service. The sync service is responsible for listening for new blocks on starknet and ethereum.
    #[clap(env = "MADARA_SYNC_DISABLED", long, alias = "no-sync")]
    pub l2_sync_disabled: bool,

    // /// The block you want to start syncing from. This will most probably break your database.
    // #[clap(env = "MADARA_UNSAFE_STARTING_BLOCK", long, value_name = "BLOCK NUMBER")]
    // pub unsafe_starting_block: Option<u64>,

    // /// Disable state root verification. When importing a block, the state root verification is the most expensive operation.
    // /// Disabling it will mean the sync service will have a huge speed-up, at a security cost
    // // TODO(docs): explain the security cost
    // #[clap(env = "MADARA_DISABLE_ROOT", long)]
    // pub disable_root: bool,
    /// Gateway api key to avoid rate limiting (optional).
    #[clap(env = "MADARA_GATEWAY_KEY", long, value_name = "API KEY")]
    pub gateway_key: Option<String>,

    /// Feeder gateway url used to sync blocks, state updates and classes
    #[clap(env = "MADARA_GATEWAY_URL", long, value_parser = parse_url, value_name = "URL")]
    pub gateway_url: Option<Url>,

    /// The port used for nodes to make rpc calls during a warp update.
    #[arg(env = "MADARA_WARP_UPDATE_PORT_RPC", long, value_name = "WARP UPDATE PORT RPC", default_value_t = RPC_DEFAULT_PORT_ADMIN)]
    pub warp_update_port_rpc: u16,

    /// The port used for nodes to send blocks during a warp update.
    #[arg(env = "MADARA_WARP_UPDATE_PORT_FGW", long, value_name = "WARP UPDATE PORT FGW", default_value_t = FGW_DEFAULT_PORT)]
    pub warp_update_port_fgw: u16,

    /// Whether to shut down the warp update sender once the migration has completed
    #[arg(env = "MADARA_WARP_UPDATE_SHUTDOWN_SENDER", long, default_value_t = false)]
    pub warp_update_shutdown_sender: bool,

    /// Whether to shut down the warp update receiver once the migration has completed
    #[arg(env = "MADARA_WARP_UPDATE_SHUTDOWN_RECEIVER", long, default_value_t = false)]
    pub warp_update_shutdown_receiver: bool,

    // /// Polling interval, in seconds. This only affects the sync service once it has caught up with the blockchain tip.
    // #[clap(
    // 	env = "MADARA_SYNC_POLLING_INTERVAL",
    //     long,
    //     value_parser = parse_duration,
    //     default_value = "4s",
    //     value_name = "SYNC POLLING INTERVAL",
    //     help = "Set the sync polling interval (e.g., '4s', '100ms', '1min')"
    // )]
    // pub sync_polling_interval: Duration,

    // /// Pending block polling interval, in seconds. This only affects the sync service once it has caught up with the blockchain tip.
    // #[clap(
    // 	env = "MADARA_PENDING_BLOCK_POLL_INTERVAL",
    //     long,
    //     value_parser = parse_duration,
    //     default_value = "2s",
    //     value_name = "PENDING BLOCK POLL INTERVAL",
    //     help = "Set the pending block poll interval (e.g., '2s', '500ms', '30s')"
    // )]
    // pub pending_block_poll_interval: Duration,

    // /// Disable sync polling. This currently means that the sync process will not import any more block once it has caught up with the
    // /// blockchain tip.
    // #[clap(env = "MADARA_NO_SYNC_POLLING", long)]
    // pub no_sync_polling: bool,
    /// Number of blocks to sync. May be useful for benchmarking the sync service.
    #[clap(env = "MADARA_N_BLOCKS_TO_SYNC", long, value_name = "NUMBER OF BLOCKS")]
    pub n_blocks_to_sync: Option<u64>,

    // /// Gracefully shutdown Madara once it has finished synchronizing all
    // /// blocks. This can either be once the node has caught up with the head of
    // /// the chain or when it has synced as many blocks as specified by
    // /// --n-blocks-to-sync.
    // #[clap(env = "MADARA_STOP_ON_SYNC", long, default_value_t = false)]
    // pub stop_on_sync: bool,

    // /// Periodically create a backup, for debugging purposes. Use it with `--backup-dir <PATH>`.
    // #[clap(env = "MADARA_BACKUP_EVERY_N_BLOCKS", long, value_name = "NUMBER OF BLOCKS")]
    // pub backup_every_n_blocks: Option<u64>,

    // /// Periodically flushes the database from ram to disk based on the number
    // /// of blocks synchronized since the last flush. You can set this to a
    // /// higher number depending on how fast your machine is at synchronizing
    // /// blocks and how much ram it has available.
    // ///
    // /// Be aware that blocks might still be flushed to db earlier based on the
    // /// value of --flush-every-n-seconds.
    // ///
    // /// Note that keeping this value high could lead to blocks being stored in
    // /// ram for longer periods of time before they are written to disk. This
    // /// might be an issue for chains which synchronize slowly.
    // #[clap(
    //     env = "MADARA_FLUSH_EVERY_N_BLOCKS",
    //     value_name = "FLUSH EVERY N BLOCKS",
    //     long,
    //     value_parser = clap::value_parser!(u64).range(..=10_000),
    //     default_value_t = 1_000
    // )]
    // pub flush_every_n_blocks: u64,

    // /// Periodically flushes the database from ram to disk based on the elapsed
    // /// time since the last flush. You can set this to a higher number
    // /// depending on how fast your machine is at synchronizing blocks and how
    // /// much ram it has available.
    // ///
    // /// Be aware that blocks might still be flushed to db earlier based on the
    // /// value of --flush-every-n-blocks.
    // ///
    // /// Note that keeping this value high could lead to blocks being stored in
    // /// ram for longer periods of time before they are written to disk. This
    // /// might be an issue for chains which synchronize slowly.
    // #[clap(
    //     env = "MADARA_FLUSH_EVERY_N_BLOCKS",
    //     value_name = "FLUSH EVERY N BLOCKS",
    //     long,
    //     value_parser = clap::value_parser!(u64).range(..=3_600),
    //     default_value_t = 5
    // )]
    // pub flush_every_n_seconds: u64,

    // /// Number of blocks to fetch in parallel. This only affects sync time, and
    // /// does not affect the node once it has reached the tip of the chain.
    // /// Increasing this can lead to lower sync times at the cost of higher cpu
    // /// and ram utilization.
    // #[clap(
    //     env = "MADARA_SYNC_PARALLELISM",
    //     long, value_name = "SYNC PARALLELISM",
    //     default_value_t = 10,
    //     value_parser = clap::value_parser!(u8).range(1..)
    // )]
    // pub sync_parallelism: u8,
    #[clap(env = "MADARA_P2P_SYNC", long)]
    pub p2p_sync: bool,
    // // Documentation needs to be kept in sync with [`mp_block_import::BlockValidationContext::compute_v0_13_2_hashes`].
    // /// UNSTABLE: Used for experimental p2p support. When p2p sync will be fully implemented, this field will go away,
    // /// and we will always compute v0.13.2 hashes. However, we can't verify the old pre-v0.13.2 blocks yet during sync,
    // /// so this field bridges the gap. When set, we will always trust the integrity of pre-v0.13.2 blocks during sync.
    // #[clap(long)]
    // pub compute_v0_13_2_hashes: bool,
}

impl L2SyncParams {
    pub fn create_feeder_client(&self, chain_config: Arc<ChainConfig>) -> anyhow::Result<Arc<GatewayProvider>> {
        let (gateway, feeder_gateway) = match &self.gateway_url {
            Some(url) => (
                url.join("/gateway/").expect("Error parsing url"),
                url.join("/feeder_gateway/").expect("Error parsing url"),
            ),
            None => (chain_config.gateway_url.clone(), chain_config.feeder_gateway_url.clone()),
        };

        let mut client = GatewayProvider::new(gateway, feeder_gateway);

        if let Some(api_key) = &self.gateway_key {
            client.add_header(
                HeaderName::from_static("x-throttling-bypass"),
                HeaderValue::from_str(api_key).with_context(|| "Invalid API key format")?,
            )
        }

        Ok(Arc::new(client))
    }
}
