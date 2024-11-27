use super::{block_stream_config, error::ResultExt};
use crate::{
    model::{self},
    sync_handlers::{self, ReqContext},
    MadaraP2pContext,
};
use futures::{channel::mpsc::Sender, stream, SinkExt, StreamExt};
use mp_block::{header::L1DataAvailabilityMode, MadaraBlockInfo};
use tokio::pin;

impl From<MadaraBlockInfo> for model::BlockHeadersResponse {
    fn from(val: MadaraBlockInfo) -> Self {
        model::BlockHeadersResponse {
            header_message: Some(model::block_headers_response::HeaderMessage::Header(model::SignedBlockHeader {
                block_hash: Some(val.block_hash.into()),
                parent_hash: Some(val.header.parent_block_hash.into()),
                number: val.header.block_number,
                time: val.header.block_timestamp,
                sequencer_address: Some(val.header.sequencer_address.into()),
                state_root: Some(val.header.global_state_root.into()),
                state_diff_commitment: val.header.state_diff_commitment.zip(val.header.state_diff_length).map(
                    |(commitment, state_diff_length)| model::StateDiffCommitment {
                        state_diff_length,
                        root: Some(commitment.into()),
                    },
                ),
                transactions: Some(model::Patricia {
                    n_leaves: val.header.transaction_count,
                    root: Some(val.header.transaction_commitment.into()),
                }),
                events: Some(model::Patricia {
                    n_leaves: val.header.event_count,
                    root: Some(val.header.event_commitment.into()),
                }),
                receipts: val.header.receipt_commitment.map(Into::into),
                protocol_version: val.header.protocol_version.to_string(),
                gas_price_fri: Some(val.header.l1_gas_price.strk_l1_gas_price.into()),
                gas_price_wei: Some(val.header.l1_gas_price.eth_l1_gas_price.into()),
                data_gas_price_fri: Some(val.header.l1_gas_price.strk_l1_data_gas_price.into()),
                data_gas_price_wei: Some(val.header.l1_gas_price.eth_l1_data_gas_price.into()),
                l1_data_availability_mode: match val.header.l1_da_mode {
                    L1DataAvailabilityMode::Calldata => model::L1DataAvailabilityMode::Calldata,
                    L1DataAvailabilityMode::Blob => model::L1DataAvailabilityMode::Blob,
                }
                .into(),
                signatures: vec![],
            })),
        }
    }
}

pub async fn headers_sync(
    ctx: ReqContext<MadaraP2pContext>,
    req: model::BlockHeadersRequest,
    mut out: Sender<model::BlockHeadersResponse>,
) -> Result<(), sync_handlers::Error> {
    let stream = ctx
    .app_ctx
    .backend
    .block_info_stream(block_stream_config(&ctx.app_ctx.backend, req.iteration.unwrap_or_default())?)
    .map(|res| res.map(Into::into))
    // Add the Fin message
    .chain(stream::once(async {
        Ok(model::BlockHeadersResponse {
            header_message: Some(model::block_headers_response::HeaderMessage::Fin(model::Fin {})),
        })
    }));

    pin!(stream);
    while let Some(res) = stream.next().await {
        if let Err(_closed) = out.send(res.or_internal_server_error("Error while reading from block stream")?).await {
            break;
        }
    }

    Ok(())
}
