use crate::{
    handlers_impl::{
        block_stream_config,
        error::{OptionExt, ResultExt},
    },
    model,
    sync_handlers::{self, ReqContext},
    MadaraP2pContext,
};
use base64::prelude::*;
use futures::{channel::mpsc::Sender, SinkExt, Stream, StreamExt};
use mc_db::db_block_id::DbBlockId;
use mp_class::{
    ClassInfo, ClassInfoWithHash, CompressedLegacyContractClass, EntryPointsByType, FlattenedSierraClass,
    LegacyClassInfo, LegacyContractEntryPoint, LegacyEntryPointsByType, SierraClassInfo, SierraEntryPoint,
};
use mp_state_update::DeclaredClassCompiledClass;
use starknet_core::types::Felt;
use std::{
    collections::{hash_map, HashMap},
    sync::Arc,
};
use tokio::pin;

use super::FromModelError;

impl model::class::Class {
    pub fn parse_model(self, compiled_class_hash: DeclaredClassCompiledClass) -> Result<ClassInfo, FromModelError> {
        Ok(match self {
            Self::Cairo0(tx) => {
                if compiled_class_hash != DeclaredClassCompiledClass::Legacy {
                    return Err(FromModelError::invalid_field("Expected Sierra class, got Legacy"));
                }
                ClassInfo::Legacy(tx.try_into()?)
            }
            Self::Cairo1(tx) => {
                let DeclaredClassCompiledClass::Sierra(compiled_class_hash) = compiled_class_hash else {
                    return Err(FromModelError::invalid_field("Expected Legacy class, got Sierra"));
                };
                ClassInfo::Sierra(tx.parse_model(compiled_class_hash)?)
            }
        })
    }
}

impl TryFrom<model::Cairo0Class> for LegacyClassInfo {
    type Error = FromModelError;
    fn try_from(value: model::Cairo0Class) -> Result<Self, Self::Error> {
        let abi: Vec<starknet_core::types::LegacyContractAbiEntry> =
            serde_json::from_str(&value.abi).map_err(FromModelError::LegacyClassJsonError)?;
        Ok(Self {
            contract_class: Arc::new(CompressedLegacyContractClass {
                program: BASE64_STANDARD.decode(&value.program).map_err(FromModelError::LegacyClassBase64Decode)?,
                entry_points_by_type: LegacyEntryPointsByType {
                    constructor: value.constructors.into_iter().map(TryInto::try_into).collect::<Result<_, _>>()?,
                    external: value.externals.into_iter().map(TryInto::try_into).collect::<Result<_, _>>()?,
                    l1_handler: value.l1_handlers.into_iter().map(TryInto::try_into).collect::<Result<_, _>>()?,
                },
                abi: Some(abi.into_iter().map(Into::into).collect()),
            }),
        })
    }
}

impl TryFrom<model::EntryPoint> for LegacyContractEntryPoint {
    type Error = FromModelError;
    fn try_from(value: model::EntryPoint) -> Result<Self, Self::Error> {
        Ok(Self {
            offset: value.offset,
            selector: value.selector.ok_or(FromModelError::missing_field("EntryPoint::selector"))?.into(),
        })
    }
}

impl model::Cairo1Class {
    pub fn parse_model(self, compiled_class_hash: Felt) -> Result<SierraClassInfo, FromModelError> {
        Ok(SierraClassInfo {
            contract_class: Arc::new(FlattenedSierraClass {
                sierra_program: self.program.into_iter().map(Into::into).collect(),
                contract_class_version: self.contract_class_version,
                entry_points_by_type: self.entry_points.unwrap_or_default().try_into()?,
                abi: self.abi,
            }),
            compiled_class_hash,
        })
    }
}

impl TryFrom<model::Cairo1EntryPoints> for EntryPointsByType {
    type Error = FromModelError;
    fn try_from(value: model::Cairo1EntryPoints) -> Result<Self, Self::Error> {
        Ok(Self {
            constructor: value.constructors.into_iter().map(TryInto::try_into).collect::<Result<_, _>>()?,
            external: value.externals.into_iter().map(TryInto::try_into).collect::<Result<_, _>>()?,
            l1_handler: value.l1_handlers.into_iter().map(TryInto::try_into).collect::<Result<_, _>>()?,
        })
    }
}

impl TryFrom<model::SierraEntryPoint> for SierraEntryPoint {
    type Error = FromModelError;
    fn try_from(value: model::SierraEntryPoint) -> Result<Self, Self::Error> {
        Ok(Self {
            selector: value.selector.ok_or(FromModelError::missing_field("SierraEntryPoint::selector"))?.into(),
            function_idx: value.index,
        })
    }
}

impl From<ClassInfo> for model::class::Class {
    fn from(value: ClassInfo) -> Self {
        match value {
            ClassInfo::Sierra(info) => Self::Cairo1(info.into()),
            ClassInfo::Legacy(info) => Self::Cairo0(info.into()),
        }
    }
}

impl From<SierraClassInfo> for model::Cairo1Class {
    fn from(value: SierraClassInfo) -> Self {
        let contract_class = Arc::unwrap_or_clone(value.contract_class);
        Self {
            // TODO(p2p-perf): we should replace the generated DTO with a hand-written one to avoid the copy of the whole class into memory.
            abi: contract_class.abi,
            entry_points: Some(contract_class.entry_points_by_type.into()),
            program: contract_class.sierra_program.into_iter().map(Into::into).collect(),
            contract_class_version: contract_class.contract_class_version,
        }
    }
}

impl From<EntryPointsByType> for model::Cairo1EntryPoints {
    fn from(value: EntryPointsByType) -> Self {
        Self {
            externals: value.external.into_iter().map(Into::into).collect(),
            l1_handlers: value.l1_handler.into_iter().map(Into::into).collect(),
            constructors: value.constructor.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<SierraEntryPoint> for model::SierraEntryPoint {
    fn from(value: SierraEntryPoint) -> Self {
        Self { index: value.function_idx, selector: Some(value.selector.into()) }
    }
}

impl From<LegacyClassInfo> for model::Cairo0Class {
    fn from(value: LegacyClassInfo) -> Self {
        let contract_class = Arc::unwrap_or_clone(value.contract_class);
        Self {
            // TODO(dto-faillible-conversion)
            abi: serde_json::to_string(&contract_class.abi().expect("Serializing contract class ABI"))
                .expect("Serializing contract class ABI to json"),
            externals: contract_class.entry_points_by_type.external.into_iter().map(Into::into).collect(),
            l1_handlers: contract_class.entry_points_by_type.l1_handler.into_iter().map(Into::into).collect(),
            constructors: contract_class.entry_points_by_type.constructor.into_iter().map(Into::into).collect(),
            program: BASE64_STANDARD.encode(&contract_class.program),
        }
    }
}

impl From<LegacyContractEntryPoint> for model::EntryPoint {
    fn from(value: LegacyContractEntryPoint) -> Self {
        Self { selector: Some(value.selector.into()), offset: value.offset }
    }
}

pub async fn classes_sync(
    ctx: ReqContext<MadaraP2pContext>,
    req: model::ClassesRequest,
    mut out: Sender<model::ClassesResponse>,
) -> Result<(), sync_handlers::Error> {
    let iterator_config = block_stream_config(&ctx.app_ctx.backend, req.iteration.unwrap_or_default())?;
    let ite = ctx
        .app_ctx
        .backend
        .block_info_iterator(iterator_config.clone());

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
