use crate::client::ClientTrait;
use anyhow::Result;
use mc_db::l1_db::LastSyncedEventBlock;
use mc_db::MadaraBackend;
use mc_mempool::Mempool;
use starknet_api::core::{ChainId, Nonce};
use starknet_types_core::felt::Felt;
use std::sync::Arc;
use tracing::{error, info};

pub mod sync;

#[cfg(test)]
mod tests;

// Defining a common message processor struct with
// processing methods.
// This is to keep the common code out of the trait implementations.
pub struct MessageProcessor<'a, T: ?Sized> {
    client: &'a T,
    backend: Arc<MadaraBackend>,
    chain_id: ChainId,
    mempool: Arc<Mempool>,
}

impl<'a, T: ClientTrait> MessageProcessor<'a, T> {
    pub fn new(client: &'a T, backend: Arc<MadaraBackend>, chain_id: ChainId, mempool: Arc<Mempool>) -> Self {
        Self { client, backend, chain_id, mempool }
    }

    pub async fn process_event(
        &self,
        event: &T::EventStruct,
        block_number: Option<u64>,
        event_index: Option<u64>,
        transaction_hash: Option<String>,
        from_address: Option<String>,
    ) -> Result<()> {
        let tx = self.client.parse_handle_message_transaction(event)?;
        let tx_nonce = tx.nonce;

        // Skip if already processed
        if self.backend.has_l1_messaging_nonce(tx_nonce)? {
            info!("Event already processed");
            return Ok(());
        }

        info!(
            "Processing Message from block: {:?}, transaction_hash: {:?}, fromAddress: {:?}",
            block_number,
            transaction_hash.unwrap(),
            from_address.unwrap(),
        );

        // Check message hash and cancellation
        let event_hash = self.client.get_messaging_hash(event)?;
        info!("Checking for cancellation, event hash: {:?}", Felt::from_bytes_be_slice(event_hash.as_slice()));

        let cancellation_timestamp = self.client.get_l1_to_l2_message_cancellations(event_hash).await?;
        if cancellation_timestamp != Felt::ZERO {
            info!("Message was cancelled in block at timestamp: {:?}", cancellation_timestamp);
            self.handle_cancelled_message(tx_nonce)?;
            return Ok(());
        }

        // Process message
        match self
            .client
            .process_message(&self.backend, event, &block_number, &event_index, &self.chain_id, self.mempool.clone())
            .await
        {
            Ok(Some(tx_hash)) => {
                info!("Message from block: {:?} submitted, transaction hash: {:?}", block_number, tx_hash);

                // Update last synced block if available
                if let (Some(block_num), Some(evt_idx)) = (block_number, event_index) {
                    let block_sent = LastSyncedEventBlock::new(block_num, evt_idx);
                    self.backend.messaging_update_last_synced_l1_block_with_event(block_sent)?;
                }
            }
            Ok(None) => {}
            Err(e) => {
                error!("Unexpected error while processing Message from block: {:?}, error: {:?}", block_number, e);
                return Err(e);
            }
        }

        Ok(())
    }

    fn handle_cancelled_message(&self, nonce: Nonce) -> Result<()> {
        match self.backend.has_l1_messaging_nonce(nonce) {
            Ok(false) => {
                self.backend.set_l1_messaging_nonce(nonce)?;
            }
            Ok(true) => {}
            Err(e) => {
                error!("Unexpected DB error: {:?}", e);
                return Err(e.into());
            }
        }
        Ok(())
    }
}

// Extension trait for easy processor creation
pub trait MessageProcessingExt: ClientTrait {
    fn message_processor(
        &self,
        backend: Arc<MadaraBackend>,
        chain_id: ChainId,
        mempool: Arc<Mempool>,
    ) -> MessageProcessor<Self>
    where
        Self: Sized,
    {
        MessageProcessor::new(self, backend, chain_id, mempool)
    }
}

// Blanket implementation for all ClientTrait implementors
impl<T: ClientTrait> MessageProcessingExt for T {}
