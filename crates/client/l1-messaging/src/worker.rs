use crate::contract::parse_handle_l1_message_transaction;
use alloy::primitives::Address;
use blockifier::transaction::transactions::L1HandlerTransaction as BlockifierL1HandlerTransaction;
use anyhow::Context;
use dc_db::{messaging_db::LastSyncedEventBlock, DeoxysBackend};
use dc_eth::client::StarknetCoreContract::LogMessageToL2;
use dc_eth::client::{EthereumClient, StarknetCoreContract};
use dp_utils::channel_wait_or_graceful_shutdown;
use futures::StreamExt;
use starknet_api::transaction::{Fee, TransactionHash};
use starknet_api::hash::StarkFelt;
use starknet_types_core::felt::Felt;
use url::Url;

pub async fn sync(
    backend: &DeoxysBackend,
    l1_url: Url,
    l1_core_address: Address,
    _chain_id: Felt,
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

            match process_l1_message(backend, &event, &meta.block_number, &meta.log_index, _chain_id).await {
                Ok(Some(tx_hash)) => {log::info!(
                    "⟠ L1 Message from block: {:?}, transaction_hash: {:?}, log_index: {:?} submitted, \
                     transaction hash on L2: {:?}",
                    meta.block_number,
                    meta.transaction_hash,
                    meta.log_index,
                    tx_hash
                );}
                Ok(None) => {}
                Err(e) => {log::error!(
                    "⟠ Unexpected error while processing L1 Message from block: {:?}, transaction_hash: {:?}, \
                     log_index: {:?}, error: {:?}",
                    meta.block_number,
                    meta.transaction_hash,
                    meta.log_index,
                    e
                )}
            }
        }
    }

    Ok(())
}

async fn process_l1_message(
    backend: &DeoxysBackend,
    event: &LogMessageToL2,
    l1_block_number: &Option<u64>,
    event_index: &Option<u64>,
    _chain_id: Felt,
) -> anyhow::Result<Option<TransactionHash>> {
    let transaction = parse_handle_l1_message_transaction(event)?;

    // Ensure that L1 message has not been executed 
    match backend.messaging_update_nonces_if_not_used(transaction.nonce) {
        Ok(true) => {},
        Ok(false) => {
            log::debug!("⟠ Event already processed: {:?}", transaction);
            return Ok(None);
        }
        Err(e) => {
            log::error!("⟠ Unexpected DB error: {:?}", e);
            return Err(e.into())
        }
    };

    let _blockifier_transaction: BlockifierL1HandlerTransaction = BlockifierL1HandlerTransaction {
        tx: transaction,
        tx_hash: TransactionHash(StarkFelt::ZERO),
        paid_fee_on_l1: Fee(event.fee.try_into()?),
    };


    // TODO: submit tx to mempool

    // TODO: remove unwraps
    backend.messaging_update_last_synced_l1_block_with_event(LastSyncedEventBlock::new(l1_block_number.unwrap(), event_index.unwrap()))?;

    Ok(None)
}
