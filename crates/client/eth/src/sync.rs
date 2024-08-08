use crate::client::EthereumClient;
use crate::l1_gas_price::{gas_price_worker, L1GasPrices};
use crate::state_update::state_update_worker;
use starknet_types_core::felt::Felt;

// use crate::cli::SyncParams;
use alloy::primitives::Address;
use anyhow::Context;
use dc_db::db_metrics::DbMetrics;
use dc_db::{DatabaseService, DeoxysBackend};
use dc_metrics::MetricsRegistry;
// use dc_sync::fetch::fetchers::FetchConfig;
// use dc_sync::metrics::block_metrics::BlockMetrics;
// use dc_telemetry::TelemetryHandle;
use dp_utils::service::Service;
use futures::lock::Mutex;
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinSet;

pub async fn sync(
    backend: &DeoxysBackend,
    eth_client: &EthereumClient,
    chain_id: Felt,
    l1_gas_prices: Arc<Mutex<L1GasPrices>>,
) -> anyhow::Result<()> {
    let state_update_fut = async { state_update_worker(backend, eth_client, chain_id).await };
    let gas_price_fut = async { gas_price_worker(eth_client, l1_gas_prices, true).await };

    tokio::try_join!(state_update_fut, gas_price_fut)?;

    Ok(())
}
