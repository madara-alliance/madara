use super::{
    block_stream_config,
    error::{OptionExt, ResultExt},
};
use crate::{
    sync_handlers::{self, ReqContext},
    MadaraP2pContext,
};
use futures::{channel::mpsc::Sender, SinkExt, Stream, StreamExt};
use mp_block::{BlockHeaderWithSignatures, ConsensusSignature};
use mp_proto::model;
use starknet_core::types::Felt;
use tokio::pin;

pub async fn headers_sync(
    ctx: ReqContext<MadaraP2pContext>,
    req: model::BlockHeadersRequest,
    mut out: Sender<model::BlockHeadersResponse>,
) -> Result<(), sync_handlers::Error> {
    let iterator_config = block_stream_config(&ctx.app_ctx.backend, req.iteration.unwrap_or_default())?;
    let ite = ctx.app_ctx.backend.block_info_iterator(iterator_config.clone());

    tracing::debug!("serving headers sync! {iterator_config:?}");

    for res in ite {
        let header = res.or_internal_server_error("Error while reading from block stream")?;
        let header = BlockHeaderWithSignatures {
            header: header.header,
            block_hash: header.block_hash,
            consensus_signatures: vec![ConsensusSignature { r: Felt::ONE, s: Felt::ONE }],
        };
        out.send(model::BlockHeadersResponse {
            header_message: Some(model::block_headers_response::HeaderMessage::Header(header.into())),
        })
        .await?;
    }

    // Add the Fin message
    out.send(model::BlockHeadersResponse {
        header_message: Some(model::block_headers_response::HeaderMessage::Fin(model::Fin {})),
    })
    .await?;

    Ok(())
}

pub async fn read_headers_stream(
    res: impl Stream<Item = model::BlockHeadersResponse>,
) -> Result<BlockHeaderWithSignatures, sync_handlers::Error> {
    pin!(res);

    let Some(res) = res.next().await else { return Err(sync_handlers::Error::EndOfStream) };
    let header = match res.header_message.ok_or_bad_request("No message")? {
        model::block_headers_response::HeaderMessage::Header(message) => message,
        model::block_headers_response::HeaderMessage::Fin(_) => {
            return Err(sync_handlers::Error::EndOfStream);
        }
    };
    BlockHeaderWithSignatures::try_from(header).or_bad_request("Converting header")
}
