use crate::client::ClientTrait;
use mc_db::MadaraBackend;
use mc_mempool::Mempool;
use mp_utils::service::ServiceContext;
use starknet_api::core::ChainId;
use starknet_types_core::felt::Felt;
use std::sync::Arc;

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

pub async fn sync<C, E>(
    settlement_client: Arc<Box<dyn ClientTrait<Config = C, EventStruct = E>>>,
    backend: Arc<MadaraBackend>,
    chain_id: ChainId,
    mempool: Arc<Mempool>,
    ctx: ServiceContext,
) -> anyhow::Result<()> {
    tracing::info!("⟠ Starting L1 Messages Syncing...");

    let last_synced_event_block = match backend.messaging_last_synced_l1_block_with_event() {
        Ok(Some(blk)) => blk,
        Ok(None) => {
            unreachable!("Should never be None")
        }
        Err(e) => {
            tracing::error!("⟠ Madara Messaging DB unavailable: {:?}", e);
            return Err(e.into());
        }
    };

    settlement_client.listen_for_messaging_events(backend, ctx, last_synced_event_block, chain_id, mempool).await?;

    Ok(())
}
