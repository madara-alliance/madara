use std::fmt;

use anyhow::{bail, Context};
use async_trait::async_trait;
use mp_utils::serde::{deserialize_url, serialize_url};
use reqwest::Url;
use serde::{Deserialize, Serialize};

use crate::Oracle;

pub const DEFAULT_API_URL: &str = "https://api.dev.pragma.build/node/v1/data/";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PragmaOracle {
    #[serde(
        default = "default_oracle_api_url",
        serialize_with = "serialize_url",
        deserialize_with = "deserialize_url"
    )]
    pub api_url: Url,
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
    fn get_fetch_url(&self, base: String, quote: String) -> String {
        format!("{}{}/{}?interval={}&aggregation={}", self.api_url, base, quote, self.interval, self.aggregation_method)
    }

    fn is_in_bounds(&self, price: u128) -> bool {
        self.price_bounds.low <= price && price <= self.price_bounds.high
    }
}

#[async_trait]
impl Oracle for PragmaOracle {
    /// Methods to retrieve STRK/ETH price from Pragma Oracle
    ///
    /// Return values:
    /// Ok((u128, u32)) : return the price tuple as (price, decimals)
    /// Err(e) : return an error if anything went wrong in the fetching process or STRK/ETH price is 0
    async fn fetch_strk_per_eth(&self) -> anyhow::Result<(u128, u32)> {
        let response = reqwest::Client::new()
            .get(self.get_fetch_url(String::from("strk"), String::from("eth")))
            .header("x-api-key", self.api_key.clone())
            .send()
            .await
            .context("failed to retrieve price from pragma oracle")?;

        let oracle_api_response = response.json::<PragmaApiResponse>().await.context("failed to parse api response")?;
        let strk_eth_price = u128::from_str_radix(oracle_api_response.price.trim_start_matches("0x"), 16)
            .context("failed to parse price")?;
        if strk_eth_price == 0 {
            bail!("Pragma api returned 0 for STRK/ETH price");
        }
        if !self.is_in_bounds(strk_eth_price) {
            bail!("STRK/ETH price outside of bounds");
        }
        Ok((strk_eth_price, oracle_api_response.decimals))
    }
}

#[derive(Default, Debug, Serialize, Deserialize, Clone)]
/// Supported Aggregation Methods
#[serde(rename_all = "snake_case")]
pub enum AggregationMethod {
    /// Computes the median value from the data.
    Median,
    /// Computes the mean (average) value from the data.
    Mean,
    /// Time Weighted Average Price. This is the default option.
    #[default]
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

fn default_oracle_api_url() -> Url {
    // safe unwrap because its parsed from a const
    Url::parse(DEFAULT_API_URL).unwrap()
}

#[derive(Deserialize, Debug)]
struct PragmaApiResponse {
    price: String,
    decimals: u32,
}

pub struct PragmaOracleBuilder {
    api_url: Url,
    api_key: String,
}

impl Default for PragmaOracleBuilder {
    fn default() -> Self {
        Self { api_url: Url::parse("about:blank").expect("valid URL"), api_key: String::default() }
    }
}

impl PragmaOracleBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_api_url(mut self, api_url: Url) -> Self {
        self.api_url = api_url;
        self
    }

    pub fn with_api_key(mut self, api_key: String) -> Self {
        self.api_key = api_key;
        self
    }

    pub fn build(self) -> PragmaOracle {
        PragmaOracle {
            api_url: self.api_url,
            api_key: self.api_key,
            aggregation_method: AggregationMethod::default(),
            interval: Interval::default(),
            price_bounds: PriceBounds::default(),
        }
    }
}
