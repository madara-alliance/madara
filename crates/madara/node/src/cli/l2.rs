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

    /// The block you want to start syncing from. This will most probably break your database.
    #[clap(env = "MADARA_UNSAFE_STARTING_BLOCK", long, value_name = "BLOCK NUMBER")]
    pub unsafe_starting_block: Option<u64>,

    /// Disable the global tries computation.
    /// When importing a block, the state root computation is the most expensive operation.
    /// Disabling it will mean a big speed-up in syncing speed, but storage proofs will be
    /// unavailable, and producing blocks will fail to compute the state root.
    #[clap(env = "MADARA_DISABLE_TRIES", long)]
    pub disable_tries: bool,

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

    /// Stop sync at a specific block_n. May be useful for benchmarking the sync service.
    #[clap(env = "MADARA_N_BLOCKS_TO_SYNC", long, value_name = "BLOCK NUMBER")]
    pub sync_stop_at: Option<u64>,

    /// Gracefully shutdown Madara once it has finished synchronizing all
    /// blocks. This can either be once the node has caught up with the head of
    /// the chain or when it has synced to the target height by using
    /// `--sync-stop-at <BLOCK NUMBER>`.
    #[clap(env = "MADARA_STOP_ON_SYNC", long)]
    pub stop_on_sync: bool,

    /// Disable pending block sync. Does not apply to p2p sync.
    #[clap(env = "MADARA_STOP_NO_PENDING_SYNC", long)]
    pub no_pending_sync: bool,

    #[clap(env = "MADARA_P2P_SYNC", long)]
    pub p2p_sync: bool,

    /// Compute pre-v0.13.2 hashes. This mode will allow the feeder gateway to display the legacy
    /// commitment hashes. Serving blocks over peer-to-peer requires >v0.13.2 hashes, which is why this
    /// is disabled by default. By enabling it, the blocks served by this block over peer-to-peer will be
    /// incorrect.
    /// This option has no effect when `--p2p-sync` is enabled, as only post-v0.13.2 commitments can be
    /// verified and saved in this mode.
    #[clap(long)]
    pub keep_pre_v0_13_2_hashes: bool,
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
