use std::collections::HashMap;

use starknet_api::contract_class::{ClassInfo as ApiClassInfo, SierraVersion};

use crate::{
    ConvertedClass, LegacyContractEntryPoint, LegacyConvertedClass, LegacyEntryPointsByType, SierraConvertedClass,
};

impl From<&ConvertedClass> for ApiClassInfo {
    fn from(converted_class: &ConvertedClass) -> Self {
        match converted_class {
            ConvertedClass::Legacy(legacy) => legacy.into(),
            ConvertedClass::Sierra(sierra) => sierra.into(),
        }
    }
}

impl From<&LegacyConvertedClass> for ApiClassInfo {
    // TODO: remove unwrap
    fn from(converted_class: &LegacyConvertedClass) -> Self {
        ApiClassInfo {
            contract_class: starknet_api::contract_class::ContractClass::V0(
                converted_class.info.contract_class.to_starknet_api_no_abi().unwrap(),
            ),
            sierra_program_length: 0,
            abi_length: 0,
            sierra_version: SierraVersion::DEPRECATED,
        }
    }
}

impl From<&SierraConvertedClass> for ApiClassInfo {
    // TODO: remove unwrap
    fn from(converted_class: &SierraConvertedClass) -> Self {
        ApiClassInfo {
            contract_class: starknet_api::contract_class::ContractClass::V1((
                converted_class.compiled.to_casm().unwrap(),
                converted_class.info.contract_class.sierra_version().unwrap(),
            )),
            sierra_program_length: converted_class.info.contract_class.program_length(),
            abi_length: converted_class.info.contract_class.abi_length(),
            sierra_version: converted_class.info.contract_class.sierra_version().unwrap(),
        }
    }
}

impl From<LegacyEntryPointsByType>
    for HashMap<
        starknet_api::contract_class::EntryPointType,
        Vec<starknet_api::deprecated_contract_class::EntryPointV0>,
    >
{
    fn from(entry_points_by_type: LegacyEntryPointsByType) -> Self {
        HashMap::from([
            (
                starknet_api::contract_class::EntryPointType::Constructor,
                entry_points_by_type.constructor.into_iter().map(|entry_point| entry_point.into()).collect(),
            ),
            (
                starknet_api::contract_class::EntryPointType::External,
                entry_points_by_type.external.into_iter().map(|entry_point| entry_point.into()).collect(),
            ),
            (
                starknet_api::contract_class::EntryPointType::L1Handler,
                entry_points_by_type.l1_handler.into_iter().map(|entry_point| entry_point.into()).collect(),
            ),
        ])
    }
}

impl From<LegacyContractEntryPoint> for starknet_api::deprecated_contract_class::EntryPointV0 {
    fn from(entry_point: LegacyContractEntryPoint) -> Self {
        starknet_api::deprecated_contract_class::EntryPointV0 {
            selector: starknet_api::core::EntryPointSelector(entry_point.selector),
            offset: starknet_api::deprecated_contract_class::EntryPointOffset(entry_point.offset as _),
        }
    }
}
