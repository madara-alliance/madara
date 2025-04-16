use alloy::primitives::private::derive_more::FromStr;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::time::Duration;
use url::Url;

use mp_utils::parsers::{parse_duration, parse_url};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum MadaraSettlementLayer {
    Eth,
    Starknet,
}

impl FromStr for MadaraSettlementLayer {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_uppercase().as_str() {
            "ETH" => Ok(MadaraSettlementLayer::Eth),
            "STARKNET" => Ok(MadaraSettlementLayer::Starknet),
            _ => Err(format!("Invalid settlement layer: {}", s)),
        }
    }
}

impl fmt::Display for MadaraSettlementLayer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MadaraSettlementLayer::Eth => write!(f, "ETH"),
            MadaraSettlementLayer::Starknet => write!(f, "STARKNET"),
        }
    }
}

#[derive(Clone, Debug, clap::Args, Deserialize, Serialize)]
pub struct L1SyncParams {
    /// Disable L1 sync.
    #[clap(env = "MADARA_SYNC_L1_DISABLED", long, alias = "no-l1-sync", conflicts_with = "l1_endpoint")]
    pub l1_sync_disabled: bool,

    /// The L1 rpc endpoint url for state verification.
    #[clap(env = "MADARA_L1_ENDPOINT", long, value_parser = parse_url, value_name = "ETHEREUM RPC URL")]
    pub l1_endpoint: Option<Url>,

    /// Fix the gas price. If the gas price is fixed it won't fetch the fee history from the ethereum.
    #[clap(env = "MADARA_GAS_PRICE", long, alias = "gas-price")]
    pub gas_price: Option<u64>,

    /// Fix the blob gas price. If the gas price is fixed it won't fetch the fee history from the ethereum.
    #[clap(env = "MADARA_DATA_GAS_PRICE", long, alias = "blob-gas-price")]
    pub blob_gas_price: Option<u64>,

    /// Fix the strk gas price. If the strk gas price is fixed it won't fetch eth <-> strk price from the oracle.
    #[clap(env = "MADARA_STRK_GAS_PRICE", long, alias = "strk-gas-price")]
    pub strk_gas_price: Option<u64>,

    /// Fix the strk blob gas price. If the strk blob gas price is fixed it won't fetch eth <-> strk price from the oracle.
    #[clap(env = "MADARA_STRK_DATA_GAS_PRICE", long, alias = "strk-blob-gas-price")]
    pub strk_blob_gas_price: Option<u64>,

    /// Oracle API url.
    #[clap(env = "ORACLE_URL", long, alias = "oracle-url")]
    pub oracle_url: Option<Url>,

    /// Oracle API key.
    #[clap(env = "ORACLE_API_KEY", long, alias = "oracle-api-key")]
    pub oracle_api_key: Option<String>,

    /// Time in which the gas price worker will fetch the gas price.
    #[clap(
		env = "MADARA_GAS_PRICE_POLL",
        long,
        default_value = "10s",
        value_parser = parse_duration,
    )]
    pub gas_price_poll: Duration,

    #[clap(
        env = "MADARA_SETTLEMENT_LAYER",
        long,
        default_value_t = MadaraSettlementLayer::Eth,
    )]
    pub settlement_layer: MadaraSettlementLayer,
}
