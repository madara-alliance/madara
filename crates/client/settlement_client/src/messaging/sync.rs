use crate::client::ClientTrait;
use futures::{Stream, StreamExt};
use mc_db::l1_db::LastSyncedEventBlock;
use mc_db::MadaraBackend;
use mc_mempool::{Mempool, MempoolProvider};
use mp_utils::service::ServiceContext;
use starknet_api::core::{ChainId, ContractAddress, EntryPointSelector, Nonce};
use starknet_api::transaction::{Calldata, L1HandlerTransaction, TransactionVersion};
use starknet_types_core::felt::Felt;
use std::sync::Arc;
use tracing::{error, info};

// L2 (Starknet) <-> L3 messaging format
// GitHub Ref : https://github.com/cartridge-gg/piltover/blob/saya/src/messaging/component.cairo#L85
#[derive(Clone)]
pub struct MessageSent {
    pub message_hash: Felt,
    pub from: Felt,
    pub to: Felt,
    pub selector: Felt,
    pub nonce: Felt,
    pub payload: Vec<Felt>,
}

#[derive(Clone)]
pub struct CommonMessagingEventData {
    pub from: Vec<u8>,
    pub to: Vec<u8>,
    pub selector: Vec<u8>,
    pub nonce: Vec<u8>,
    pub payload: Vec<Vec<u8>>,
    pub fee: Option<Vec<u8>>,
    pub transaction_hash: Option<Vec<u8>>,
    pub message_hash: Option<Vec<u8>>,
    pub block_number: Option<u64>,
    pub event_index: Option<u64>,
}

pub async fn sync<C, S>(
    settlement_client: Arc<Box<dyn ClientTrait<Config = C, StreamType = S>>>,
    backend: Arc<MadaraBackend>,
    chain_id: ChainId,
    mempool: Arc<Mempool>,
    mut ctx: ServiceContext,
) -> anyhow::Result<()>
where
    S: Stream<Item = Option<anyhow::Result<CommonMessagingEventData>>> + Send + 'static,
{
    info!("⟠ Starting L1 Messages Syncing...");

    let last_synced_event_block = match backend.messaging_last_synced_l1_block_with_event() {
        Ok(Some(blk)) => blk,
        Ok(None) => {
            unreachable!("Should never be None")
        }
        Err(e) => {
            error!("⟠ Madara Messaging DB unavailable: {:?}", e);
            return Err(e.into());
        }
    };

    let stream = settlement_client.get_event_stream(last_synced_event_block).await?;
    let mut event_stream = Box::pin(stream);

    while let Some(Some(event_result)) = ctx.run_until_cancelled(event_stream.next()).await {
        match event_result {
            Some(event) => {
                let event_data = event?;
                let tx = parse_handle_message_transaction(&event_data)?;
                let tx_nonce = tx.nonce;

                // Skip if already processed
                if backend.has_l1_messaging_nonce(tx_nonce)? {
                    info!("Event already processed");
                    return Ok(());
                }

                info!(
                    "Processing Message from block: {:?}, transaction_hash: {:?}, fromAddress: {:?}",
                    event_data.block_number, event_data.transaction_hash, event_data.from,
                );

                // Check message hash and cancellation
                let event_hash = settlement_client.get_messaging_hash(&event_data)?;
                info!("Checking for cancellation, event hash: {:?}", Felt::from_bytes_be_slice(event_hash.as_slice()));

                let cancellation_timestamp = settlement_client.get_l1_to_l2_message_cancellations(event_hash).await?;
                if cancellation_timestamp != Felt::ZERO {
                    info!("Message was cancelled in block at timestamp: {:?}", cancellation_timestamp);
                    handle_cancelled_message(backend, tx_nonce)?;
                    return Ok(());
                }

                // Process message
                match process_message(
                    &backend,
                    &event_data,
                    &event_data.block_number,
                    &event_data.event_index,
                    &chain_id,
                    mempool.clone(),
                )
                .await
                {
                    Ok(Some(tx_hash)) => {
                        info!(
                            "Message from block: {:?} submitted, transaction hash: {:?}",
                            event_data.block_number, tx_hash
                        );

                        // Update last synced block if available
                        if let (Some(block_num), Some(evt_idx)) = (event_data.block_number, event_data.event_index) {
                            let block_sent = LastSyncedEventBlock::new(block_num, evt_idx);
                            backend.messaging_update_last_synced_l1_block_with_event(block_sent)?;
                        }
                    }
                    Ok(None) => {}
                    Err(e) => {
                        error!(
                            "Unexpected error while processing Message from block: {:?}, error: {:?}",
                            event_data.block_number, e
                        );
                        return Err(e);
                    }
                }
            }
            None => {}
        }
    }
    Ok(())
}

fn handle_cancelled_message(backend: Arc<MadaraBackend>, nonce: Nonce) -> anyhow::Result<()> {
    match backend.has_l1_messaging_nonce(nonce) {
        Ok(false) => {
            backend.set_l1_messaging_nonce(nonce)?;
        }
        Ok(true) => {}
        Err(e) => {
            error!("Unexpected DB error: {:?}", e);
            return Err(e.into());
        }
    }
    Ok(())
}

pub fn parse_handle_message_transaction(event: &CommonMessagingEventData) -> anyhow::Result<L1HandlerTransaction> {
    // L1 from address.
    let from_address = Felt::from_bytes_be_slice(event.from.as_slice());

    // L2 contract to call.
    let contract_address = Felt::from_bytes_be_slice(event.to.as_slice());

    // Function of the contract to call.
    let entry_point_selector = Felt::from_bytes_be_slice(event.selector.as_slice());

    // L1 message nonce.
    let nonce = Felt::from_bytes_be_slice(event.nonce.as_slice());

    let event_payload: Vec<Felt> =
        event.payload.clone().into_iter().map(|ele| Felt::from_bytes_be_slice(ele.as_slice())).collect();

    let calldata: Calldata = {
        let mut calldata: Vec<_> = Vec::with_capacity(event.payload.len() + 1);
        calldata.push(from_address);
        calldata.extend(event_payload);
        Calldata(Arc::new(calldata))
    };

    Ok(L1HandlerTransaction {
        nonce: Nonce(nonce),
        contract_address: ContractAddress(contract_address.try_into()?),
        entry_point_selector: EntryPointSelector(entry_point_selector),
        calldata,
        version: TransactionVersion(Felt::ZERO),
    })
}

async fn process_message(
    backend: &MadaraBackend,
    event: &CommonMessagingEventData,
    settlement_layer_block_number: &Option<u64>,
    event_index: &Option<u64>,
    _chain_id: &ChainId,
    mempool: Arc<Mempool>,
) -> anyhow::Result<Option<Felt>> {
    let transaction = parse_handle_message_transaction(event)?;
    let tx_nonce = transaction.nonce;
    let fees = vec_to_u128_be(event.fee.clone().unwrap());

    // Ensure that L1 message has not been executed
    match backend.has_l1_messaging_nonce(tx_nonce) {
        Ok(false) => {
            backend.set_l1_messaging_nonce(tx_nonce)?;
        }
        Ok(true) => {
            tracing::debug!("⟠ Event already processed: {:?}", transaction);
            return Ok(None);
        }
        Err(e) => {
            error!("⟠ Unexpected DB error: {:?}", e);
            return Err(e.into());
        }
    };

    let res = mempool.accept_l1_handler_tx(transaction.into(), fees.unwrap_or(0))?;

    // TODO: remove unwraps
    // Ques: shall it panic if no block number of event_index?
    let block_sent = LastSyncedEventBlock::new(settlement_layer_block_number.unwrap(), event_index.unwrap());
    backend.messaging_update_last_synced_l1_block_with_event(block_sent)?;

    Ok(Some(res.transaction_hash))
}

fn vec_to_u128_be(bytes: Vec<u8>) -> Option<u128> {
    if bytes.len() > 16 {
        return None;
    }
    // Pad with zeros if less than 16 bytes
    let mut padded = vec![0u8; 16];
    padded[16 - bytes.len()..].copy_from_slice(&bytes);
    Some(u128::from_be_bytes(padded.try_into().unwrap()))
}
