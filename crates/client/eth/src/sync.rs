use crate::client::EthereumClient;
use crate::l1_gas_price::gas_price_worker;
use crate::l1_messaging::sync;
use crate::state_update::{state_update_worker, L1HeadSender};
use mc_mempool::{GasPriceProvider, Mempool};
use starknet_api::core::ChainId;
use std::sync::Arc;
use std::time::Duration;

use mc_db::MadaraBackend;

pub async fn l1_sync_worker(
    backend: &MadaraBackend,
    eth_client: &EthereumClient,
    chain_id: ChainId,
    l1_gas_provider: GasPriceProvider,
    gas_price_sync_disabled: bool,
    gas_price_poll_ms: Duration,
    mempool: Arc<Mempool>,
    l1_head_sender: L1HeadSender,
) -> anyhow::Result<()> {
    tokio::try_join!(
        state_update_worker(backend, eth_client, chain_id.clone(), l1_head_sender),
        async {
            if !gas_price_sync_disabled {
                gas_price_worker(eth_client, l1_gas_provider, gas_price_poll_ms).await?;
            }
            Ok(())
        },
        sync(backend, eth_client, &chain_id, mempool)
    )?;

    Ok(())
}
