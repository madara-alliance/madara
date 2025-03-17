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
use mp_class::{ClassInfo, ClassInfoWithHash};
use mp_proto::model;
use mp_state_update::DeclaredClassCompiledClass;
use starknet_core::types::Felt;
use std::collections::{hash_map, HashMap};
use tokio::pin;

pub async fn classes_sync(
    ctx: ReqContext<MadaraP2pContext>,
    req: model::ClassesRequest,
    mut out: Sender<model::ClassesResponse>,
) -> Result<(), sync_handlers::Error> {
    let iterator_config = block_stream_config(&ctx.app_ctx.backend, req.iteration.unwrap_or_default())?;
    let ite = ctx.app_ctx.backend.block_info_iterator(iterator_config.clone());

    tracing::debug!("serving classes sync! {iterator_config:?}");

    for res in ite {
        let header = res.or_internal_server_error("Error while reading from block stream")?;

        let state_diff = ctx
            .app_ctx
            .backend
            .get_block_state_diff(&DbBlockId::Number(header.header.block_number))
            .or_internal_server_error("Getting block state diff")?
            .ok_or_internal_server_error("No state diff for block")?;

        for class_hash in state_diff
            .deprecated_declared_classes
            .into_iter()
            .chain(state_diff.declared_classes.into_iter().map(|entry| entry.class_hash))
        {
            let Some(class_info) = ctx
                .app_ctx
                .backend
                .get_class_info(&DbBlockId::Number(header.header.block_number), &class_hash)
                .or_internal_server_error("Getting class info")?
            else {
                continue; // it is possible that we have the state diff but not the class yet for that block.
            };

            let class = model::Class { domain: 0, class_hash: Some(class_hash.into()), class: Some(class_info.into()) };

            out.send(model::ClassesResponse {
                class_message: Some(model::classes_response::ClassMessage::Class(class)),
            })
            .await?
        }
    }

    // Add the Fin message
    let _res = out
        .send(model::ClassesResponse { class_message: Some(model::classes_response::ClassMessage::Fin(model::Fin {})) })
        .await;

    Ok(())
}

/// Note: you need to get the `declared_classes` field from the `StateDiff` beforehand.
pub async fn read_classes_stream(
    res: impl Stream<Item = model::ClassesResponse>,
    declared_classes: &HashMap<Felt, DeclaredClassCompiledClass>,
) -> Result<Vec<ClassInfoWithHash>, sync_handlers::Error> {
    pin!(res);

    let mut out: HashMap<Felt, ClassInfo> = HashMap::with_capacity(declared_classes.len());
    while out.len() < declared_classes.len() {
        let handle_fin = || {
            if out.is_empty() {
                sync_handlers::Error::EndOfStream
            } else {
                sync_handlers::Error::bad_request(format!(
                    "Expected {} messages in stream, got {}",
                    declared_classes.len(),
                    out.len()
                ))
            }
        };

        let Some(res) = res.next().await else { return Err(handle_fin()) };
        let val = match res.class_message.ok_or_bad_request("No message")? {
            model::classes_response::ClassMessage::Class(message) => message,
            model::classes_response::ClassMessage::Fin(_) => return Err(handle_fin()),
        };

        let class_hash = val.class_hash.ok_or_bad_request("Missing class_hash field")?.into();
        let hash_map::Entry::Vacant(out_entry) = out.entry(class_hash) else {
            return Err(sync_handlers::Error::bad_request("Duplicate class_hash"));
        };

        // Get the expected compiled_class_hash.
        let Some(compiled_class_hash) = declared_classes.get(&class_hash) else {
            return Err(sync_handlers::Error::bad_request("Duplicate class_hash"));
        };

        let class_info = val
            .class
            .ok_or_bad_request("Missing class field")?
            .parse_model(*compiled_class_hash)
            .or_bad_request("Converting class info")?;
        out_entry.insert(class_info);
    }

    Ok(out.into_iter().map(|(class_hash, class_info)| ClassInfoWithHash { class_info, class_hash }).collect())
}
