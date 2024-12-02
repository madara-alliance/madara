use std::fmt;

use anyhow::{bail, Context};
use serde::{Deserialize, Serialize};

pub const DEFAULT_API_URL: &str = "https://api.dev.pragma.build/node/v1/data/";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PragmaOracle {
    #[serde(default = "default_oracle_api_url")]
    pub api_url: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub aggregation_method: AggregationMethod,
    #[serde(default)]
    pub interval: Interval,
    #[serde(default)]
    pub price_bounds: PriceBounds,
}

impl Default for PragmaOracle {
    fn default() -> Self {
        Self {
            api_url: default_oracle_api_url(),
            api_key: String::default(),
            aggregation_method: AggregationMethod::Median,
            interval: Interval::OneMinute,
            price_bounds: Default::default(),
        }
    }
}

impl PragmaOracle {
    pub fn new(api_url: String, api_key: String) -> Self {
        Self {
            api_url,
            api_key,
            aggregation_method: AggregationMethod::Median,
            interval: Interval::OneMinute,
            price_bounds: Default::default(),
        }
    }

    pub fn get_fetch_url(&self, base: String, quote: String) -> String {
        format!("{}{}/{}?interval={}&aggregation={}", self.api_url, base, quote, self.interval, self.aggregation_method)
    }

    pub async fn fetch_eth_strk_price(&self) -> anyhow::Result<(u128, u32)> {
        let response = reqwest::Client::new()
            .get(self.get_fetch_url(String::from("eth"), String::from("strk")))
            .header("x-api-key", self.api_key.clone())
            .send()
            .await
            .context("failed to retrieve price from pragma oracle")?;

        let oracle_api_response = response.json::<PragmaApiResponse>().await.context("failed to parse api response")?;
        let eth_strk_price = u128::from_str_radix(oracle_api_response.price.trim_start_matches("0x"), 16).context("failed to parse price")?;
        if eth_strk_price ==  0 {
            bail!("Pragma api returned 0 for eth/strk price");
        }
        Ok((eth_strk_price, oracle_api_response.decimals))
    }
}

#[derive(Default, Debug, Serialize, Deserialize, Clone)]
/// Supported Aggregation Methods
#[serde(rename_all = "snake_case")]
pub enum AggregationMethod {
    Median,
    Mean,
    #[default]
    // Time weighted average price
    Twap,
}

impl fmt::Display for AggregationMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            AggregationMethod::Median => "median",
            AggregationMethod::Mean => "mean",
            AggregationMethod::Twap => "twap",
        };
        write!(f, "{}", name)
    }
}

/// Supported Aggregation Intervals
#[derive(Default, Debug, Serialize, Deserialize, Clone)]
pub enum Interval {
    #[serde(rename = "1min")]
    OneMinute,
    #[serde(rename = "15min")]
    FifteenMinutes,
    #[serde(rename = "1h")]
    OneHour,
    #[serde(rename = "2h")]
    #[default]
    TwoHours,
}

impl fmt::Display for Interval {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Interval::OneMinute => "1min",
            Interval::FifteenMinutes => "15min",
            Interval::OneHour => "1h",
            Interval::TwoHours => "2h",
        };
        write!(f, "{}", name)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceBounds {
    pub low: u128,
    pub high: u128,
}

impl Default for PriceBounds {
    fn default() -> Self {
        Self { low: 0, high: u128::MAX }
    }
}

fn default_oracle_api_url() -> String {
    DEFAULT_API_URL.into()
}

#[derive(Deserialize, Debug)]
struct PragmaApiResponse {
    price: String,
    decimals: u32,
}
