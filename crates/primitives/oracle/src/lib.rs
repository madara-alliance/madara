use anyhow::bail;
use reqwest::Url;
use serde::{Deserialize, Serialize};

mod pragma;

use pragma::*;

/// Wrapper enum for different oracle providers.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "oracle_name", content = "config")]
pub enum Oracle {
    Pragma(PragmaOracle),
}

impl Oracle {
    pub fn new(oracle_name: &str, url: Url, key: String) -> anyhow::Result<Self> {
        match oracle_name {
            "Pragma" => Ok(Oracle::Pragma(PragmaOracle::new(url, key))),
            _ => bail!("Unknown Oracle name"),
        }
    }

    pub fn set_base_url(&mut self, url: Url) -> &mut Self {
        match self {
            Oracle::Pragma(pragma_oracle) => pragma_oracle.api_url = url,
        }
        self
    }

    pub async fn fetch_eth_strk_price(&self) -> anyhow::Result<(u128, u32)> {
        match self {
            Oracle::Pragma(pragma_oracle) => pragma_oracle.fetch_eth_strk_price().await,
        }
    }

    pub fn set_api_key(&mut self, key: String) -> &mut Self {
        match self {
            Oracle::Pragma(pragma_oracle) => pragma_oracle.api_key = key,
        }
        self
    }

    pub fn get_fetch_url(&self, base: String, quote: String) -> String {
        match self {
            Oracle::Pragma(pragma_oracle) => pragma_oracle.get_fetch_url(base, quote),
        }
    }

    pub fn get_api_key(&self) -> &String {
        match self {
            Oracle::Pragma(oracle) => &oracle.api_key,
        }
    }

    pub fn is_in_bounds(&self, price: u128) -> bool {
        match self {
            Oracle::Pragma(oracle) => oracle.price_bounds.low <= price && price <= oracle.price_bounds.high,
        }
    }
}

impl Default for Oracle {
    fn default() -> Self {
        Self::Pragma(PragmaOracle::default())
    }
}
