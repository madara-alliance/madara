use super::error::ResultExt;
use crate::{
    handlers_impl::{block_stream_config, error::OptionExt},
    model,
    sync_handlers::{self, ReqContext},
    MadaraP2pContext,
};
use futures::{channel::mpsc::Sender, SinkExt, Stream, StreamExt};
use mc_db::db_block_id::DbBlockId;
use mp_state_update::{
    ContractStorageDiffItem, DeclaredClassItem, NonceUpdate, ReplacedClassItem, StateDiff, StorageEntry,
};
use starknet_core::types::Felt;
use std::collections::{BTreeMap, HashMap};
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
    let ite = ctx
        .app_ctx
        .backend
        .block_info_iterator(block_stream_config(&ctx.app_ctx.backend, req.iteration.unwrap_or_default())?);

    tracing::debug!("state diffs sync!");

    for res in ite {
        let header = res.or_internal_server_error("Error while reading from block stream")?;

        let Some(state_diff) = ctx
            .app_ctx
            .backend
            .get_block_state_diff(&DbBlockId::Number(header.header.block_number))
            .or_internal_server_error("Getting block state diff")?
        else {
            continue; // it is possible that we have the header but not the state diff for this block yet.
        };

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

/// Note: The declared_contracts field of the state diff will be empty. Its content will be instead in the replaced_classes field.
pub async fn read_state_diffs_stream(
    res: impl Stream<Item = model::StateDiffsResponse>,
    state_diff_length: usize,
) -> Result<StateDiff, sync_handlers::Error> {
    pin!(res);

    let mut storage_diffs: BTreeMap<Felt, BTreeMap<Felt, Felt>> = BTreeMap::new();
    let mut all_declared_classes: BTreeMap<Felt, Option<Felt>> = BTreeMap::new(); // Sierra and legacy.
    let mut deployed_or_replaced_contracts: BTreeMap<Felt, Felt> = BTreeMap::new();
    let mut nonces: BTreeMap<Felt, Felt> = BTreeMap::new();

    let mut current_len: usize = 0;
    while current_len < state_diff_length {
        let old_len = current_len;

        let handle_fin = || {
            if current_len == 0 {
                sync_handlers::Error::EndOfStream
            } else {
                sync_handlers::Error::bad_request(format!(
                    "Expected {} messages in stream, got {}",
                    state_diff_length, current_len
                ))
            }
        };
        let Some(res) = res.next().await else { return Err(handle_fin()) };
        match res.state_diff_message.ok_or_bad_request("No message")? {
            model::state_diffs_response::StateDiffMessage::ContractDiff(message) => {
                let contract = message.address.ok_or_bad_request("Missing field address in contract diff")?.into();
                if let Some(nonce) = message.nonce.map(Into::into) {
                    if nonces.insert(contract, nonce).is_some() {
                        return Err(sync_handlers::Error::bad_request("Duplicate nonce"));
                    }
                    current_len += 1;
                }
                if let Some(nonce) = message.class_hash.map(Into::into) {
                    if deployed_or_replaced_contracts.insert(contract, nonce).is_some() {
                        return Err(sync_handlers::Error::bad_request(
                            "Duplicate deployed contract or replaced contract class",
                        ));
                    }
                    current_len += 1;
                }
                // ignore message.domain for now.
                if !message.values.is_empty() {
                    let entry = storage_diffs.entry(contract).or_default();
                    for kv in message.values {
                        let key = kv.key.ok_or_bad_request("No storage key for storage diff")?.into();
                        if entry.insert(key, kv.value.unwrap_or_default().into()).is_some() {
                            return Err(sync_handlers::Error::bad_request("Duplicate contract storage value diff"));
                        }
                        current_len += 1;
                    }
                }
            }
            model::state_diffs_response::StateDiffMessage::DeclaredClass(message) => {
                let class_hash = message.class_hash.ok_or_bad_request("No class hash for declared class")?.into();
                if all_declared_classes.insert(class_hash, message.compiled_class_hash.map(Into::into)).is_some() {
                    return Err(sync_handlers::Error::bad_request("Duplicate contract storage value diff"));
                }
                current_len += 1;
            }
            model::state_diffs_response::StateDiffMessage::Fin(_) => return Err(handle_fin()),
        };

        // Just in case contract_diff message is actually empty, we want to force the peer to actually make progress.
        // Otherwise they could just keep the stream hanging by sending empty messages.
        if old_len == current_len {
            return Err(sync_handlers::Error::bad_request("Empty state diff message"));
        }
    }

    let mut deprecated_declared_classes = vec![];
    let mut declared_classes = vec![];

    for (class_hash, compiled_class_hash) in all_declared_classes {
        if let Some(compiled_class_hash) = compiled_class_hash {
            declared_classes.push(DeclaredClassItem { class_hash, compiled_class_hash });
        } else {
            deprecated_declared_classes.push(class_hash);
        }
    }

    let state_diff = StateDiff {
        storage_diffs: storage_diffs
            .into_iter()
            .map(|(address, kv)| ContractStorageDiffItem {
                address,
                storage_entries: kv.into_iter().map(|(key, value)| StorageEntry { key, value }).collect(),
            })
            .collect(),
        deprecated_declared_classes,
        declared_classes,
        deployed_contracts: Default::default(), // TODO: i really want to drop the support of that field.
        replaced_classes: deployed_or_replaced_contracts
            .into_iter()
            .map(|(contract_address, class_hash)| ReplacedClassItem { contract_address, class_hash })
            .collect(),
        nonces: nonces.into_iter().map(|(contract_address, nonce)| NonceUpdate { contract_address, nonce }).collect(),
    };

    if state_diff.len() != state_diff_length {
        // this shouldn't happen since we always check for duplicates when inserting, but just in case.
        return Err(sync_handlers::Error::bad_request("State diff length mismatch"));
    }

    Ok(state_diff)
}
