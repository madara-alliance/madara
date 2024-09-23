use url::Url;
const DEFAULT_GAS_PRICE_POLL_MS: u64 = 10_000;

fn parse_url(s: &str) -> Result<Url, url::ParseError> {
    s.parse()
}

#[derive(Clone, Debug, clap::Args)]
pub struct L1SyncParams {
    /// Disable L1 sync.
    #[clap(long, alias = "no-l1-sync", conflicts_with = "l1_endpoint")]
    pub sync_l1_disabled: bool,

    /// The L1 rpc endpoint url for state verification.
    #[clap(long, value_parser = parse_url, value_name = "ETHEREUM RPC URL")]
    pub l1_endpoint: Option<Url>,

    /// Disable the gas price sync service. The sync service is responsible to fetch the fee history from the ethereum.
    #[clap(long, alias = "no-gas-price-sync")]
    pub gas_price_sync_disabled: bool,

    /// Time in milliseconds in which the gas price worker will fetch the gas price.
    #[clap(long, default_value_t = DEFAULT_GAS_PRICE_POLL_MS, alias = "gas-price-poll")]
    pub gas_price_poll_ms: u64,
}
