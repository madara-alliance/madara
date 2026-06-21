//! Price oracle abstractions for Madara.
//!
//! Defines the [`Oracle`] trait used to fetch the STRK/ETH price, together with
//! the [Pragma](pragma) oracle implementation.
#![warn(missing_docs)]

use async_trait::async_trait;
use mp_convert::FixedPoint;

/// [Pragma](https://www.pragma.build/) oracle implementation of [`Oracle`].
pub mod pragma;

/// A price oracle capable of returning the current STRK/ETH price.
#[async_trait]
pub trait Oracle: Send + Sync {
    /// Fetches the current STRK-per-ETH price as a fixed-point value.
    async fn fetch_strk_per_eth(&self) -> anyhow::Result<FixedPoint>;
}
