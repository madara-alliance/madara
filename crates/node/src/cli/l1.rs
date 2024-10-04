use std::time::Duration;

use url::Url;

use mp_utils::parsers::{parse_duration, parse_url};

#[derive(Clone, Debug, clap::Args)]
pub struct L1SyncParams {
    /// Disable L1 sync.
    #[clap(env = "MADARA_SYNC_L1_DISABLED", long, alias = "no-l1-sync", conflicts_with = "l1_endpoint")]
    pub sync_l1_disabled: bool,

    /// The L1 rpc endpoint url for state verification.
    #[clap(env = "MADARA_L1_ENDPOINT", long, value_parser = parse_url, value_name = "ETHEREUM RPC URL")]
    pub l1_endpoint: Option<Url>,

    /// Disable the gas price sync service. The sync service is responsible to fetch the fee history from the ethereum.
    #[clap(env = "MADARA_GAS_PRICE_SYNC_DISABLED", long, alias = "no-gas-price-sync")]
    pub gas_price_sync_disabled: bool,

    /// Time in which the gas price worker will fetch the gas price.
    #[clap(
		env = "MADARA_GAS_PRICE_POLL",
        long,
        default_value = "10s",
        value_parser = parse_duration,
    )]
    pub gas_price_poll: Duration,
}
