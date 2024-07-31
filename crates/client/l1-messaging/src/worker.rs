use std::sync::Arc;

use crate::contract::parse_handle_l1_message_transaction;
use alloy::{
    primitives::{Address, U256},
    providers::{Provider, ProviderBuilder},
};
use anyhow::Context;
use dc_db::DeoxysBackend;
use dc_eth::client::StarknetCoreContract::LogMessageToL2;
use dc_eth::client::{EthereumClient, StarknetCoreContract};
use dp_utils::channel_wait_or_graceful_shutdown;
use futures::StreamExt;
use starknet_api::transaction::{Fee, TransactionHash};
use starknet_types_core::felt::Felt;
use url::Url;

pub async fn sync(
    backend: &DeoxysBackend,
    l1_url: Url,
    l1_core_address: Address,
    chain_id: Felt,
) -> anyhow::Result<()> {
    let client = EthereumClient::new(l1_url, l1_core_address).await.context("Creating ethereum client")?;

    log::info!("⟠ Starting L1 Messages Syncing...");

    let last_synced_event_block = match backend.messaging_last_synced_l1_block_with_event() {
        Ok(Some(blk)) => blk,
        Ok(None) => {
            unreachable!("Should never be None")
        }
        Err(e) => {
            log::error!("⟠ Madara Messaging DB unavailable: {:?}", e);
            return Err(e.into());
        }
    };

    let event_filter = client.l1_core_contract.event_filter::<StarknetCoreContract::LogMessageToL2>();
    let mut event_stream = event_filter
        .from_block(last_synced_event_block.block_number)
        .watch()
        .await
        .context("Failed to watch event filter")?
        .into_stream();

    while let Some(event_result) = channel_wait_or_graceful_shutdown(event_stream.next()).await {
        if let Ok((event, meta)) = event_result {
            log::info!(
                "⟠ Processing L1 Message from block: {:?}, transaction_hash: {:?}, log_index: {:?}",
                meta.block_number,
                meta.transaction_hash,
                meta.log_index
            );

            match process_l1_message(backend, &event).await {
                Ok(Some(tx_hash)) => {}
                Ok(None) => {}
                Err(e) => {}
            }
        }
    }

    Ok(())
}

// pub struct L1HandlerTransaction {
//     pub tx: starknet_api::transaction::L1HandlerTransaction,
//     pub tx_hash: TransactionHash,
//     pub paid_fee_on_l1: Fee,
// }

async fn process_l1_message(
    backend: &DeoxysBackend,
    event: &LogMessageToL2,
) -> anyhow::Result<Option<TransactionHash>> {
    let paid_fee_on_l1 = event.fee;
    let nonce = event.nonce;

    let transaction = parse_handle_l1_message_transaction(event);

    // TODO: submit tx to mempool

    todo!();
}
