use crate::Oracle;
use anyhow::{bail, Context};
use async_trait::async_trait;
use mp_convert::FixedPoint;
use mp_utils::serde::{deserialize_url, serialize_url};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use std::fmt;

/// Default Pragma API base URL used when none is configured.
pub const DEFAULT_API_URL: &str = "https://api.dev.pragma.build/node/v1/data/";

/// Configuration and client for the Pragma price oracle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PragmaOracle {
    /// Base URL of the Pragma API.
    #[serde(
        default = "default_oracle_api_url",
        serialize_with = "serialize_url",
        deserialize_with = "deserialize_url"
    )]
    pub api_url: Url,
    /// API key sent in the `x-api-key` header.
    #[serde(default)]
    pub api_key: String,
    /// Aggregation method requested from the API.
    #[serde(default)]
    pub aggregation_method: AggregationMethod,
    /// Aggregation interval requested from the API.
    #[serde(default)]
    pub interval: Interval,
    /// Acceptable price bounds; prices outside this range are rejected.
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
    /// Ok(FixedPoint) : return the price tuple as (price, decimals)
    /// Err(e) : return an error if anything went wrong in the fetching process or STRK/ETH price is 0
    async fn fetch_strk_per_eth(&self) -> anyhow::Result<FixedPoint> {
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
        Ok(FixedPoint::new(strk_eth_price, oracle_api_response.decimals))
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
    /// One-minute aggregation interval.
    #[serde(rename = "1min")]
    OneMinute,
    /// Fifteen-minute aggregation interval.
    #[serde(rename = "15min")]
    FifteenMinutes,
    /// One-hour aggregation interval.
    #[serde(rename = "1h")]
    OneHour,
    /// Two-hour aggregation interval. This is the default option.
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

/// Inclusive lower/upper bounds a fetched price must fall within to be accepted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceBounds {
    /// Inclusive lower bound.
    pub low: u128,
    /// Inclusive upper bound.
    pub high: u128,
}

impl Default for PriceBounds {
    fn default() -> Self {
        Self { low: 0, high: u128::MAX }
    }
}

fn default_oracle_api_url() -> Url {
    Url::parse(DEFAULT_API_URL).expect("DEFAULT_API_URL is a valid constant url")
}

#[derive(Deserialize, Debug)]
struct PragmaApiResponse {
    price: String,
    decimals: u32,
}

/// Builder for [`PragmaOracle`], allowing the API URL and key to be set
/// incrementally before constructing the oracle with default aggregation
/// settings.
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
    /// Creates a new builder with default (blank) values.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the Pragma API base URL.
    pub fn with_api_url(mut self, api_url: Url) -> Self {
        self.api_url = api_url;
        self
    }

    /// Sets the Pragma API key.
    pub fn with_api_key(mut self, api_key: String) -> Self {
        self.api_key = api_key;
        self
    }

    /// Builds the [`PragmaOracle`] with default aggregation method, interval and price bounds.
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
