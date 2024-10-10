use std::time::Duration;

use url::Url;

use mp_utils::parsers::{parse_duration, parse_url};

#[derive(Clone, Debug, clap::Args)]
pub struct L1SyncParams {
    /// Disable L1 sync.
    #[clap(long, alias = "no-l1-sync", conflicts_with = "l1_endpoint")]
    pub sync_l1_disabled: bool,

    /// The L1 rpc endpoint url for state verification.
    #[clap(long, value_parser = parse_url, value_name = "ETHEREUM RPC URL")]
    pub l1_endpoint: Option<Url>,

    /// Fix the gas price. If the gas price is fixed it won't fetch the fee history from the ethereum.
    #[clap(long, alias = "gas-price")]
    pub gas_price: Option<u64>,

    /// Fix the blob gas price. If the gas price is fixed it won't fetch the fee history from the ethereum.
    #[clap(long, alias = "blob-gas-price")]
    pub blob_gas_price: Option<u64>,

    /// Time in which the gas price worker will fetch the gas price.
    #[clap(
        long,
        default_value = "10s",
        value_parser = parse_duration,
    )]
    pub gas_price_poll: Duration,
}
