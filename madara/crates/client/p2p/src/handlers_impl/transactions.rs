//! TODO: range check contract addresses?

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
use mp_block::TransactionWithReceipt;
use mp_proto::model;
use tokio::pin;

/// Reply to a transactions sync request.
pub async fn transactions_sync(
    ctx: ReqContext<MadaraP2pContext>,
    req: model::TransactionsRequest,
    mut out: Sender<model::TransactionsResponse>,
) -> Result<(), sync_handlers::Error> {
    let iterator_config = block_stream_config(&ctx.app_ctx.backend, req.iteration.unwrap_or_default())?;
    let ite = ctx.app_ctx.backend.block_info_iterator(iterator_config.clone());

    tracing::debug!("serving transactions sync! {iterator_config:?}");

    for res in ite {
        let header = res.or_internal_server_error("Error while reading from block stream")?;

        let Some(block_inner) = ctx
            .app_ctx
            .backend
            .get_block_inner(&DbBlockId::Number(header.header.block_number))
            .or_internal_server_error("Getting block state diff")?
        else {
            continue; // it is possible that we have the header but not the transactions for this block yet.
        };

        for (transaction, receipt) in block_inner.transactions.into_iter().zip(block_inner.receipts) {
            let el = TransactionWithReceipt { transaction, receipt };

            out.send(model::TransactionsResponse {
                transaction_message: Some(model::transactions_response::TransactionMessage::TransactionWithReceipt(
                    el.into(),
                )),
            })
            .await?
        }
    }

    // Add the Fin message
    out.send(model::TransactionsResponse {
        transaction_message: Some(model::transactions_response::TransactionMessage::Fin(model::Fin {})),
    })
    .await?;

    Ok(())
}

/// Used by [`crate::commands::P2pCommands::make_transactions_stream`] to send a transactions stream request.
/// Note that the events in the transaction receipt will not be filled in, as they need to be fetched using the events stream request.
pub async fn read_transactions_stream(
    res: impl Stream<Item = model::TransactionsResponse>,
    transactions_count: usize,
) -> Result<Vec<TransactionWithReceipt>, sync_handlers::Error> {
    pin!(res);

    let mut vec = Vec::with_capacity(transactions_count);
    for i in 0..transactions_count {
        let handle_fin = || {
            if i == 0 {
                sync_handlers::Error::EndOfStream
            } else {
                sync_handlers::Error::bad_request(format!(
                    "Expected {} messages in stream, got {}",
                    transactions_count, i
                ))
            }
        };

        let Some(res) = res.next().await else { return Err(handle_fin()) };
        let val = match res.transaction_message.ok_or_bad_request("No message")? {
            model::transactions_response::TransactionMessage::TransactionWithReceipt(message) => message,
            model::transactions_response::TransactionMessage::Fin(_) => return Err(handle_fin()),
        };
        let res = TransactionWithReceipt::try_from(val).or_bad_request("Converting transaction with receipt")?;
        vec.push(res);
    }

    Ok(vec)
}
