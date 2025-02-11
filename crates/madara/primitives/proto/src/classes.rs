use crate::{model, FromModelError};
use base64::prelude::*;
use mp_class::{
    ClassInfo, CompressedLegacyContractClass, EntryPointsByType, FlattenedSierraClass, LegacyClassInfo,
    LegacyContractEntryPoint, LegacyEntryPointsByType, SierraClassInfo, SierraEntryPoint,
};
use mp_state_update::DeclaredClassCompiledClass;
use starknet_core::types::Felt;
use std::sync::Arc;

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
