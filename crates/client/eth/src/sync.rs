use crate::client::EthereumClient;
use crate::l1_gas_price::gas_price_worker;
use crate::state_update::state_update_worker;
use mc_mempool::GasPriceProvider;
use starknet_types_core::felt::Felt;
use std::time::Duration;

use mc_db::MadaraBackend;

pub async fn l1_sync_worker(
    backend: &MadaraBackend,
    eth_client: &EthereumClient,
    chain_id: Felt,
    l1_gas_provider: GasPriceProvider,
    gas_price_sync_disabled: bool,
    gas_price_poll_ms: Duration,
) -> anyhow::Result<()> {
    tokio::try_join!(state_update_worker(backend, eth_client, chain_id), async {
        if !gas_price_sync_disabled {
            gas_price_worker(eth_client, l1_gas_provider, gas_price_poll_ms).await?;
        }
        Ok(())
    })?;

    Ok(())
}
