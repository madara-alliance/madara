use crate::{
    handlers_impl::{
        block_stream_config,
        error::{OptionExt, ResultExt},
    },
    model,
    sync_handlers::{self, ReqContext},
    MadaraP2pContext,
};
use futures::{channel::mpsc::Sender, SinkExt, StreamExt};
use mc_db::db_block_id::DbBlockId;
use tokio::pin;

pub async fn events_sync(
    ctx: ReqContext<MadaraP2pContext>,
    req: model::EventsRequest,
    mut out: Sender<model::EventsResponse>,
) -> Result<(), sync_handlers::Error> {
    let stream = ctx
        .app_ctx
        .backend
        .block_info_stream(block_stream_config(&ctx.app_ctx.backend, req.iteration.unwrap_or_default())?);
    pin!(stream);

    tracing::debug!("events sync!");

    while let Some(res) = stream.next().await {
        let header = res.or_internal_server_error("Error while reading from block stream")?;

        let block_inner = ctx
            .app_ctx
            .backend
            .get_block_inner(&DbBlockId::Number(header.header.block_number))
            .or_internal_server_error("Getting block state diff")?
            .ok_or_internal_server_error("No body for block")?;

        for (transaction_hash, event) in block_inner
            .receipts
            .iter()
            .zip(&header.tx_hashes)
            .flat_map(|(receipt, &tx_hash)| receipt.events().iter().cloned().map(move |ev| (tx_hash, ev)))
        {
            let event = model::Event {
                transaction_hash: Some(transaction_hash.into()),
                from_address: Some(event.from_address.into()),
                keys: event.keys.into_iter().map(Into::into).collect(),
                data: event.data.into_iter().map(Into::into).collect(),
            };

            out.send(model::EventsResponse { event_message: Some(model::events_response::EventMessage::Event(event)) })
                .await?;
        }
    }

    // Add the Fin message
    out.send(model::EventsResponse { event_message: Some(model::events_response::EventMessage::Fin(model::Fin {})) })
        .await?;

    Ok(())
}
