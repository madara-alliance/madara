use std::sync::Arc;

use anyhow::Context;
use mc_gateway::client::builder::FeederClient;
use mp_chain_config::ChainConfig;
use mp_utils::parsers::parse_url;
use reqwest::header::{HeaderName, HeaderValue};
use url::Url;

#[derive(Clone, Debug, clap::Args)]
pub struct SyncParams {
    /// Disable the sync service. The sync service is responsible for listening for new blocks on starknet and ethereum.
    #[clap(env = "MADARA_SYNC_DISABLED", long, alias = "no-sync")]
    pub sync_disabled: bool,

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

    /// Number of blocks to sync. May be useful for benchmarking the sync service.
    #[clap(env = "MADARA_N_BLOCKS_TO_SYNC", long, value_name = "NUMBER OF BLOCKS")]
    pub n_blocks_to_sync: Option<u64>,

    /// Periodically create a backup, for debugging purposes. Use it with `--backup-dir <PATH>`.
    #[clap(env = "MADARA_BACKUP_EVERY_N_BLOCKS", long, value_name = "NUMBER OF BLOCKS")]
    pub backup_every_n_blocks: Option<u64>,

    #[clap(env = "MADARA_P2P_SYNC", long)]
    pub p2p_sync: bool,
    // // Documentation needs to be kept in sync with [`mp_block_import::BlockValidationContext::compute_v0_13_2_hashes`].
    // /// UNSTABLE: Used for experimental p2p support. When p2p sync will be fully implemented, this field will go away,
    // /// and we will always compute v0.13.2 hashes. However, we can't verify the old pre-v0.13.2 blocks yet during sync,
    // /// so this field bridges the gap. When set, we will always trust the integrity of pre-v0.13.2 blocks during sync.
    // #[clap(long)]
    // pub compute_v0_13_2_hashes: bool,
}

impl SyncParams {
    pub fn create_feeder_client(&self, chain_config: Arc<ChainConfig>) -> anyhow::Result<FeederClient> {
        let (gateway, feeder_gateway) = match &self.gateway_url {
            Some(url) => (
                url.join("/gateway/").expect("Error parsing url"),
                url.join("/feeder_gateway/").expect("Error parsing url"),
            ),
            None => (chain_config.gateway_url.clone(), chain_config.feeder_gateway_url.clone()),
        };

        let mut client = FeederClient::new(gateway, feeder_gateway);

        if let Some(api_key) = &self.gateway_key {
            client.add_header(
                HeaderName::from_static("x-throttling-bypass"),
                HeaderValue::from_str(api_key).with_context(|| "Invalid API key format")?,
            )
        }

        Ok(client)
    }
}
