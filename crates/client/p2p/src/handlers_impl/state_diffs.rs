use super::error::ResultExt;
use crate::{
    handlers_impl::{block_stream_config, error::OptionExt},
    model,
    sync_handlers::{self, ReqContext},
    MadaraP2pContext,
};
use futures::{channel::mpsc::Sender, SinkExt, StreamExt};
use mc_db::db_block_id::DbBlockId;
use mp_state_update::StateDiff;
use starknet_core::types::Felt;
use std::collections::HashMap;
use tokio::pin;

fn contract_state_diffs(state_diff: &StateDiff) -> HashMap<Felt, model::ContractDiff> {
    let mut res: HashMap<Felt, model::ContractDiff> = Default::default();

    for nonce_update in &state_diff.nonces {
        let entry = res.entry(nonce_update.contract_address).or_default();
        entry.nonce = Some(nonce_update.nonce.into());
    }

    for deployed_contract in &state_diff.deployed_contracts {
        let entry = res.entry(deployed_contract.address).or_default();
        entry.class_hash = Some(deployed_contract.class_hash.into());
    }

    for replaced_class in &state_diff.replaced_classes {
        let entry = res.entry(replaced_class.contract_address).or_default();
        entry.class_hash = Some(replaced_class.class_hash.into());
    }

    for storage_diff in &state_diff.storage_diffs {
        let entry = res.entry(storage_diff.address).or_default();
        entry.values = storage_diff
            .storage_entries
            .iter()
            .map(|el| model::ContractStoredValue { key: Some(el.key.into()), value: Some(el.value.into()) })
            .collect();
    }

    res
}

pub async fn state_diffs_sync(
    ctx: ReqContext<MadaraP2pContext>,
    req: model::StateDiffsRequest,
    mut out: Sender<model::StateDiffsResponse>,
) -> Result<(), sync_handlers::Error> {
    let stream = ctx
        .app_ctx
        .backend
        .block_info_stream(block_stream_config(&ctx.app_ctx.backend, req.iteration.unwrap_or_default())?);
    pin!(stream);

    tracing::debug!("state diffs sync!");

    while let Some(res) = stream.next().await {
        let header = res.or_internal_server_error("Error while reading from block stream")?;

        let state_diff = ctx
            .app_ctx
            .backend
            .get_block_state_diff(&DbBlockId::Number(header.header.block_number))
            .or_internal_server_error("Getting block state diff")?
            .ok_or_internal_server_error("No state diff for block")?;

        // Legacy declared classes
        for &class_hash in &state_diff.deprecated_declared_classes {
            let el = model::DeclaredClass { class_hash: Some(class_hash.into()), compiled_class_hash: None };

            out.send(model::StateDiffsResponse {
                state_diff_message: Some(model::state_diffs_response::StateDiffMessage::DeclaredClass(el)),
            })
            .await?
        }

        // Declared classes
        for declared_class in &state_diff.declared_classes {
            let el = model::DeclaredClass {
                class_hash: Some(declared_class.class_hash.into()),
                compiled_class_hash: Some(declared_class.compiled_class_hash.into()),
            };

            out.send(model::StateDiffsResponse {
                state_diff_message: Some(model::state_diffs_response::StateDiffMessage::DeclaredClass(el)),
            })
            .await?
        }

        // Contract updates (nonces, storage, deployed/replaced)
        for (contract_address, mut el) in contract_state_diffs(&state_diff) {
            el.address = Some(contract_address.into());
            out.send(model::StateDiffsResponse {
                state_diff_message: Some(model::state_diffs_response::StateDiffMessage::ContractDiff(el)),
            })
            .await?
        }
    }

    // Add the Fin message
    out.send(model::StateDiffsResponse {
        state_diff_message: Some(model::state_diffs_response::StateDiffMessage::Fin(model::Fin {})),
    })
    .await?;

    Ok(())
}
