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
use futures::{channel::mpsc::Sender, SinkExt, StreamExt};
use mc_db::db_block_id::DbBlockId;
use mp_class::{
    ClassInfo, EntryPointsByType, LegacyClassInfo, LegacyContractEntryPoint, SierraClassInfo, SierraEntryPoint,
};
use std::sync::Arc;
use tokio::pin;

// impl TryFrom<model::class::Class> for ClassInfo {
//     type Error = FromModelError;
//     fn try_from(value: model::class::Class) -> Result<Self, Self::Error> {
//         use model::class::Class;
//         Ok(match value {
//             Class::Cairo0(tx) => Self::Legacy(tx.try_into()?),
//             Class::Cairo1(tx) => Self::Sierra(tx.try_into()?),
//         })
//     }
// }

// impl ToBlockifier

// impl TryFrom<model::Cairo1Class> for SierraClassInfo {
//     type Error = FromModelError;
//     fn try_from(value: model::Cairo1Class) -> Result<Self, Self::Error> {
//         Ok(
//             Self {
//                 contract_class: Arc::new(FlattenedSierraClass {
//                     sierra_program: value.program.into_iter().map(Into::into).collect(),
//                     contract_class_version: value.contract_class_version,
//                     entry_points_by_type: value.entry_points.unwrap_or_default().try_into(),
//                     abi: value.abi,
//                 }),
//                 compiled_class_hash: value.,
//             }
//         )
//     }
// }

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
    let stream = ctx
        .app_ctx
        .backend
        .block_info_stream(block_stream_config(&ctx.app_ctx.backend, req.iteration.unwrap_or_default())?);
    pin!(stream);

    tracing::debug!("classes sync!");

    while let Some(res) = stream.next().await {
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
            let class_info = ctx
                .app_ctx
                .backend
                .get_class_info(&DbBlockId::Number(header.header.block_number), &class_hash)
                .or_internal_server_error("Getting class info")?
                .ok_or_internal_server_error("Class info not found")?;

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
