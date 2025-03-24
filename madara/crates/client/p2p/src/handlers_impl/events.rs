use crate::{
    handlers_impl::{
        block_stream_config,
        error::{OptionExt, ResultExt},
    },
    sync_handlers::{self, ReqContext},
    MadaraP2pContext,
};
use futures::{channel::mpsc::Sender, SinkExt, Stream, StreamExt};
use mc_db::db_block_id::DbBlockId;
use mp_proto::model;
use mp_receipt::EventWithTransactionHash;
use tokio::pin;

pub async fn events_sync(
    ctx: ReqContext<MadaraP2pContext>,
    req: model::EventsRequest,
    mut out: Sender<model::EventsResponse>,
) -> Result<(), sync_handlers::Error> {
    let iterator_config = block_stream_config(&ctx.app_ctx.backend, req.iteration.unwrap_or_default())?;
    let ite = ctx.app_ctx.backend.block_info_iterator(iterator_config.clone());

    tracing::debug!("serving events sync! {iterator_config:?}");

    for res in ite {
        let header = res.or_internal_server_error("Error while reading from block stream")?;

        let Some(block_inner) = ctx
            .app_ctx
            .backend
            .get_block_inner(&DbBlockId::Number(header.header.block_number))
            .or_internal_server_error("Getting block state diff")?
        else {
            continue; // it is possible that we have the header but not the events for this block yet.
        };

        let events = block_inner.receipts.iter().zip(&header.tx_hashes).flat_map(|(receipt, &transaction_hash)| {
            receipt.events().iter().cloned().map(move |event| EventWithTransactionHash { transaction_hash, event })
        });
        for event in events {
            out.send(model::EventsResponse {
                event_message: Some(model::events_response::EventMessage::Event(event.into())),
            })
            .await?;
        }
    }

    // Add the Fin message
    out.send(model::EventsResponse { event_message: Some(model::events_response::EventMessage::Fin(model::Fin {})) })
        .await?;

    Ok(())
}

pub async fn read_events_stream(
    res: impl Stream<Item = model::EventsResponse>,
    events_count: usize,
) -> Result<Vec<EventWithTransactionHash>, sync_handlers::Error> {
    pin!(res);

    let mut vec = Vec::with_capacity(events_count);
    for i in 0..events_count {
        let handle_fin = || {
            if i == 0 {
                sync_handlers::Error::EndOfStream
            } else {
                sync_handlers::Error::bad_request(format!("Expected {} messages in stream, got {}", events_count, i))
            }
        };

        let Some(res) = res.next().await else { return Err(handle_fin()) };
        let val = match res.event_message.ok_or_bad_request("No message")? {
            model::events_response::EventMessage::Event(message) => message,
            model::events_response::EventMessage::Fin(_) => return Err(handle_fin()),
        };
        let res = EventWithTransactionHash::try_from(val).or_bad_request("Converting transaction with receipt")?;
        vec.push(res);
    }

    Ok(vec)
}
