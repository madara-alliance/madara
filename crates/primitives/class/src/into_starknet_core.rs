use std::sync::Arc;

use crate::{
    CompressedLegacyContractClass, ContractClass, EntryPointsByType, FlattenedSierraClass, FunctionStateMutability,
    LegacyContractAbiEntry, LegacyContractEntryPoint, LegacyEntryPointsByType, LegacyEventAbiEntry, LegacyEventAbiType,
    LegacyFunctionAbiEntry, LegacyFunctionAbiType, LegacyStructAbiEntry, LegacyStructAbiType, LegacyStructMember,
    LegacyTypedParameter, SierraEntryPoint,
};

impl From<starknet_core::types::ContractClass> for ContractClass {
    fn from(contract_class: starknet_core::types::ContractClass) -> Self {
        match contract_class {
            starknet_core::types::ContractClass::Sierra(flattened_sierra_class) => {
                ContractClass::Sierra(Arc::new(flattened_sierra_class.into()))
            }
            starknet_core::types::ContractClass::Legacy(compressed_legacy_contract_class) => {
                ContractClass::Legacy(Arc::new(compressed_legacy_contract_class.into()))
            }
        }
    }
}

impl From<ContractClass> for starknet_core::types::ContractClass {
    fn from(contract_class: ContractClass) -> Self {
        match contract_class {
            ContractClass::Sierra(flattened_sierra_class) => {
                starknet_core::types::ContractClass::Sierra((*flattened_sierra_class).clone().into())
            }
            ContractClass::Legacy(compressed_legacy_contract_class) => {
                starknet_core::types::ContractClass::Legacy((*compressed_legacy_contract_class).clone().into())
            }
        }
    }
}

impl From<starknet_core::types::FlattenedSierraClass> for FlattenedSierraClass {
    fn from(flattened_sierra_class: starknet_core::types::FlattenedSierraClass) -> Self {
        FlattenedSierraClass {
            sierra_program: flattened_sierra_class.sierra_program,
            contract_class_version: flattened_sierra_class.contract_class_version,
            entry_points_by_type: flattened_sierra_class.entry_points_by_type.into(),
            abi: flattened_sierra_class.abi,
        }
    }
}

impl From<FlattenedSierraClass> for starknet_core::types::FlattenedSierraClass {
    fn from(flattened_sierra_class: FlattenedSierraClass) -> Self {
        starknet_core::types::FlattenedSierraClass {
            sierra_program: flattened_sierra_class.sierra_program,
            contract_class_version: flattened_sierra_class.contract_class_version,
            entry_points_by_type: flattened_sierra_class.entry_points_by_type.into(),
            abi: flattened_sierra_class.abi,
        }
    }
}

impl From<starknet_core::types::EntryPointsByType> for EntryPointsByType {
    fn from(entry_points_by_type: starknet_core::types::EntryPointsByType) -> Self {
        EntryPointsByType {
            constructor: entry_points_by_type
                .constructor
                .into_iter()
                .map(|sierra_entry_point| sierra_entry_point.into())
                .collect(),
            external: entry_points_by_type
                .external
                .into_iter()
                .map(|sierra_entry_point| sierra_entry_point.into())
                .collect(),
            l1_handler: entry_points_by_type
                .l1_handler
                .into_iter()
                .map(|sierra_entry_point| sierra_entry_point.into())
                .collect(),
        }
    }
}

impl From<EntryPointsByType> for starknet_core::types::EntryPointsByType {
    fn from(entry_points_by_type: EntryPointsByType) -> Self {
        starknet_core::types::EntryPointsByType {
            constructor: entry_points_by_type
                .constructor
                .into_iter()
                .map(|sierra_entry_point| sierra_entry_point.into())
                .collect(),
            external: entry_points_by_type
                .external
                .into_iter()
                .map(|sierra_entry_point| sierra_entry_point.into())
                .collect(),
            l1_handler: entry_points_by_type
                .l1_handler
                .into_iter()
                .map(|sierra_entry_point| sierra_entry_point.into())
                .collect(),
        }
    }
}

impl From<starknet_core::types::SierraEntryPoint> for SierraEntryPoint {
    fn from(sierra_entry_point: starknet_core::types::SierraEntryPoint) -> Self {
        SierraEntryPoint { selector: sierra_entry_point.selector, function_idx: sierra_entry_point.function_idx }
    }
}

impl From<SierraEntryPoint> for starknet_core::types::SierraEntryPoint {
    fn from(sierra_entry_point: SierraEntryPoint) -> Self {
        starknet_core::types::SierraEntryPoint {
            selector: sierra_entry_point.selector,
            function_idx: sierra_entry_point.function_idx,
        }
    }
}

impl From<starknet_core::types::CompressedLegacyContractClass> for CompressedLegacyContractClass {
    fn from(compressed_legacy_contract_class: starknet_core::types::CompressedLegacyContractClass) -> Self {
        CompressedLegacyContractClass {
            program: compressed_legacy_contract_class.program,
            entry_points_by_type: compressed_legacy_contract_class.entry_points_by_type.into(),
            abi: compressed_legacy_contract_class
                .abi
                .map(|abi| abi.into_iter().map(|legacy_contract_abi_entry| legacy_contract_abi_entry.into()).collect()),
        }
    }
}

impl From<CompressedLegacyContractClass> for starknet_core::types::CompressedLegacyContractClass {
    fn from(compressed_legacy_contract_class: CompressedLegacyContractClass) -> Self {
        starknet_core::types::CompressedLegacyContractClass {
            program: compressed_legacy_contract_class.program,
            entry_points_by_type: compressed_legacy_contract_class.entry_points_by_type.into(),
            abi: compressed_legacy_contract_class
                .abi
                .map(|abi| abi.into_iter().map(|legacy_contract_abi_entry| legacy_contract_abi_entry.into()).collect()),
        }
    }
}

impl From<starknet_core::types::LegacyEntryPointsByType> for LegacyEntryPointsByType {
    fn from(legacy_entry_points_by_type: starknet_core::types::LegacyEntryPointsByType) -> Self {
        LegacyEntryPointsByType {
            constructor: legacy_entry_points_by_type
                .constructor
                .into_iter()
                .map(|legacy_contract_entry_point| legacy_contract_entry_point.into())
                .collect(),
            external: legacy_entry_points_by_type
                .external
                .into_iter()
                .map(|legacy_contract_entry_point| legacy_contract_entry_point.into())
                .collect(),
            l1_handler: legacy_entry_points_by_type
                .l1_handler
                .into_iter()
                .map(|legacy_contract_entry_point| legacy_contract_entry_point.into())
                .collect(),
        }
    }
}

impl From<LegacyEntryPointsByType> for starknet_core::types::LegacyEntryPointsByType {
    fn from(legacy_entry_points_by_type: LegacyEntryPointsByType) -> Self {
        starknet_core::types::LegacyEntryPointsByType {
            constructor: legacy_entry_points_by_type
                .constructor
                .into_iter()
                .map(|legacy_contract_entry_point| legacy_contract_entry_point.into())
                .collect(),
            external: legacy_entry_points_by_type
                .external
                .into_iter()
                .map(|legacy_contract_entry_point| legacy_contract_entry_point.into())
                .collect(),
            l1_handler: legacy_entry_points_by_type
                .l1_handler
                .into_iter()
                .map(|legacy_contract_entry_point| legacy_contract_entry_point.into())
                .collect(),
        }
    }
}

impl From<starknet_core::types::LegacyContractEntryPoint> for LegacyContractEntryPoint {
    fn from(legacy_contract_entry_point: starknet_core::types::LegacyContractEntryPoint) -> Self {
        LegacyContractEntryPoint {
            offset: legacy_contract_entry_point.offset,
            selector: legacy_contract_entry_point.selector,
        }
    }
}

impl From<LegacyContractEntryPoint> for starknet_core::types::LegacyContractEntryPoint {
    fn from(legacy_contract_entry_point: LegacyContractEntryPoint) -> Self {
        starknet_core::types::LegacyContractEntryPoint {
            offset: legacy_contract_entry_point.offset,
            selector: legacy_contract_entry_point.selector,
        }
    }
}

impl From<starknet_core::types::LegacyContractAbiEntry> for LegacyContractAbiEntry {
    fn from(legacy_contract_abi_entry: starknet_core::types::LegacyContractAbiEntry) -> Self {
        match legacy_contract_abi_entry {
            starknet_core::types::LegacyContractAbiEntry::Function(legacy_function_abi_entry) => {
                LegacyContractAbiEntry::Function(legacy_function_abi_entry.into())
            }
            starknet_core::types::LegacyContractAbiEntry::Event(legacy_event_abi_entry) => {
                LegacyContractAbiEntry::Event(legacy_event_abi_entry.into())
            }
            starknet_core::types::LegacyContractAbiEntry::Struct(legacy_struct_abi_entry) => {
                LegacyContractAbiEntry::Struct(legacy_struct_abi_entry.into())
            }
        }
    }
}

impl From<LegacyContractAbiEntry> for starknet_core::types::LegacyContractAbiEntry {
    fn from(legacy_contract_abi_entry: LegacyContractAbiEntry) -> Self {
        match legacy_contract_abi_entry {
            LegacyContractAbiEntry::Function(legacy_function_abi_entry) => {
                starknet_core::types::LegacyContractAbiEntry::Function(legacy_function_abi_entry.into())
            }
            LegacyContractAbiEntry::Event(legacy_event_abi_entry) => {
                starknet_core::types::LegacyContractAbiEntry::Event(legacy_event_abi_entry.into())
            }
            LegacyContractAbiEntry::Struct(legacy_struct_abi_entry) => {
                starknet_core::types::LegacyContractAbiEntry::Struct(legacy_struct_abi_entry.into())
            }
        }
    }
}

impl From<starknet_core::types::LegacyFunctionAbiEntry> for LegacyFunctionAbiEntry {
    fn from(legacy_function_abi_entry: starknet_core::types::LegacyFunctionAbiEntry) -> Self {
        LegacyFunctionAbiEntry {
            r#type: legacy_function_abi_entry.r#type.into(),
            name: legacy_function_abi_entry.name,
            inputs: legacy_function_abi_entry.inputs.into_iter().map(|abi_entry| abi_entry.into()).collect(),
            outputs: legacy_function_abi_entry.outputs.into_iter().map(|abi_entry| abi_entry.into()).collect(),
            state_mutability: legacy_function_abi_entry
                .state_mutability
                .map(|state_mutability| state_mutability.into()),
        }
    }
}

impl From<LegacyFunctionAbiEntry> for starknet_core::types::LegacyFunctionAbiEntry {
    fn from(legacy_function_abi_entry: LegacyFunctionAbiEntry) -> Self {
        starknet_core::types::LegacyFunctionAbiEntry {
            r#type: legacy_function_abi_entry.r#type.into(),
            name: legacy_function_abi_entry.name,
            inputs: legacy_function_abi_entry.inputs.into_iter().map(|abi_entry| abi_entry.into()).collect(),
            outputs: legacy_function_abi_entry.outputs.into_iter().map(|abi_entry| abi_entry.into()).collect(),
            state_mutability: legacy_function_abi_entry
                .state_mutability
                .map(|state_mutability| state_mutability.into()),
        }
    }
}

impl From<starknet_core::types::LegacyEventAbiEntry> for LegacyEventAbiEntry {
    fn from(legacy_event_abi_entry: starknet_core::types::LegacyEventAbiEntry) -> Self {
        LegacyEventAbiEntry {
            r#type: legacy_event_abi_entry.r#type.into(),
            name: legacy_event_abi_entry.name,
            keys: legacy_event_abi_entry.keys.into_iter().map(|abi_entry| abi_entry.into()).collect(),
            data: legacy_event_abi_entry.data.into_iter().map(|abi_entry| abi_entry.into()).collect(),
        }
    }
}

impl From<LegacyEventAbiEntry> for starknet_core::types::LegacyEventAbiEntry {
    fn from(legacy_event_abi_entry: LegacyEventAbiEntry) -> Self {
        starknet_core::types::LegacyEventAbiEntry {
            r#type: legacy_event_abi_entry.r#type.into(),
            name: legacy_event_abi_entry.name,
            keys: legacy_event_abi_entry.keys.into_iter().map(|abi_entry| abi_entry.into()).collect(),
            data: legacy_event_abi_entry.data.into_iter().map(|abi_entry| abi_entry.into()).collect(),
        }
    }
}

impl From<starknet_core::types::LegacyStructAbiEntry> for LegacyStructAbiEntry {
    fn from(legacy_struct_abi_entry: starknet_core::types::LegacyStructAbiEntry) -> Self {
        LegacyStructAbiEntry {
            r#type: legacy_struct_abi_entry.r#type.into(),
            name: legacy_struct_abi_entry.name,
            size: legacy_struct_abi_entry.size,
            members: legacy_struct_abi_entry.members.into_iter().map(|member| member.into()).collect(),
        }
    }
}

impl From<LegacyStructAbiEntry> for starknet_core::types::LegacyStructAbiEntry {
    fn from(legacy_struct_abi_entry: LegacyStructAbiEntry) -> Self {
        starknet_core::types::LegacyStructAbiEntry {
            r#type: legacy_struct_abi_entry.r#type.into(),
            name: legacy_struct_abi_entry.name,
            size: legacy_struct_abi_entry.size,
            members: legacy_struct_abi_entry.members.into_iter().map(|member| member.into()).collect(),
        }
    }
}

impl From<starknet_core::types::LegacyStructMember> for LegacyStructMember {
    fn from(legacy_struct_member: starknet_core::types::LegacyStructMember) -> Self {
        LegacyStructMember {
            name: legacy_struct_member.name,
            r#type: legacy_struct_member.r#type,
            offset: legacy_struct_member.offset,
        }
    }
}

impl From<LegacyStructMember> for starknet_core::types::LegacyStructMember {
    fn from(legacy_struct_member: LegacyStructMember) -> Self {
        starknet_core::types::LegacyStructMember {
            name: legacy_struct_member.name,
            r#type: legacy_struct_member.r#type,
            offset: legacy_struct_member.offset,
        }
    }
}

impl From<starknet_core::types::LegacyTypedParameter> for LegacyTypedParameter {
    fn from(legacy_typed_parameter: starknet_core::types::LegacyTypedParameter) -> Self {
        LegacyTypedParameter { r#type: legacy_typed_parameter.r#type, name: legacy_typed_parameter.name }
    }
}

impl From<LegacyTypedParameter> for starknet_core::types::LegacyTypedParameter {
    fn from(legacy_typed_parameter: LegacyTypedParameter) -> Self {
        starknet_core::types::LegacyTypedParameter {
            r#type: legacy_typed_parameter.r#type,
            name: legacy_typed_parameter.name,
        }
    }
}

impl From<starknet_core::types::LegacyFunctionAbiType> for LegacyFunctionAbiType {
    fn from(legacy_function_abi_type: starknet_core::types::LegacyFunctionAbiType) -> Self {
        match legacy_function_abi_type {
            starknet_core::types::LegacyFunctionAbiType::Function => LegacyFunctionAbiType::Function,
            starknet_core::types::LegacyFunctionAbiType::L1Handler => LegacyFunctionAbiType::L1Handler,
            starknet_core::types::LegacyFunctionAbiType::Constructor => LegacyFunctionAbiType::Constructor,
        }
    }
}

impl From<LegacyFunctionAbiType> for starknet_core::types::LegacyFunctionAbiType {
    fn from(legacy_function_abi_type: LegacyFunctionAbiType) -> Self {
        match legacy_function_abi_type {
            LegacyFunctionAbiType::Function => starknet_core::types::LegacyFunctionAbiType::Function,
            LegacyFunctionAbiType::L1Handler => starknet_core::types::LegacyFunctionAbiType::L1Handler,
            LegacyFunctionAbiType::Constructor => starknet_core::types::LegacyFunctionAbiType::Constructor,
        }
    }
}

impl From<starknet_core::types::LegacyEventAbiType> for LegacyEventAbiType {
    fn from(legacy_event_abi_type: starknet_core::types::LegacyEventAbiType) -> Self {
        match legacy_event_abi_type {
            starknet_core::types::LegacyEventAbiType::Event => LegacyEventAbiType::Event,
        }
    }
}

impl From<LegacyEventAbiType> for starknet_core::types::LegacyEventAbiType {
    fn from(legacy_event_abi_type: LegacyEventAbiType) -> Self {
        match legacy_event_abi_type {
            LegacyEventAbiType::Event => starknet_core::types::LegacyEventAbiType::Event,
        }
    }
}

impl From<starknet_core::types::LegacyStructAbiType> for LegacyStructAbiType {
    fn from(legacy_struct_abi_type: starknet_core::types::LegacyStructAbiType) -> Self {
        match legacy_struct_abi_type {
            starknet_core::types::LegacyStructAbiType::Struct => LegacyStructAbiType::Struct,
        }
    }
}

impl From<LegacyStructAbiType> for starknet_core::types::LegacyStructAbiType {
    fn from(legacy_struct_abi_type: LegacyStructAbiType) -> Self {
        match legacy_struct_abi_type {
            LegacyStructAbiType::Struct => starknet_core::types::LegacyStructAbiType::Struct,
        }
    }
}

impl From<starknet_core::types::FunctionStateMutability> for FunctionStateMutability {
    fn from(function_state_mutability: starknet_core::types::FunctionStateMutability) -> Self {
        match function_state_mutability {
            starknet_core::types::FunctionStateMutability::View => FunctionStateMutability::View,
        }
    }
}

impl From<FunctionStateMutability> for starknet_core::types::FunctionStateMutability {
    fn from(function_state_mutability: FunctionStateMutability) -> Self {
        match function_state_mutability {
            FunctionStateMutability::View => starknet_core::types::FunctionStateMutability::View,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        CompressedLegacyContractClass, ContractClass, EntryPointsByType, FlattenedSierraClass, FunctionStateMutability,
        LegacyContractAbiEntry, LegacyContractEntryPoint, LegacyEntryPointsByType, LegacyEventAbiEntry,
        LegacyEventAbiType, LegacyFunctionAbiEntry, LegacyFunctionAbiType, LegacyStructAbiEntry, LegacyStructAbiType,
        LegacyStructMember, LegacyTypedParameter, SierraEntryPoint,
    };
    use mp_convert::test::assert_consistent_conversion;
    use starknet_core::types::ContractClass as StarknetContractClass;
    use starknet_types_core::felt::Felt;

    #[test]
    fn test_legacy_contract_class_conversion() {
        let legacy_contract_class = CompressedLegacyContractClass {
            program: vec![1, 2, 3],
            entry_points_by_type: LegacyEntryPointsByType {
                constructor: vec![LegacyContractEntryPoint { offset: 0, selector: Felt::from(1) }],
                external: vec![LegacyContractEntryPoint { offset: 1, selector: Felt::from(2) }],
                l1_handler: vec![LegacyContractEntryPoint { offset: 2, selector: Felt::from(3) }],
            },
            abi: Some(vec![
                LegacyContractAbiEntry::Function(LegacyFunctionAbiEntry {
                    r#type: LegacyFunctionAbiType::Function,
                    name: "function".to_string(),
                    inputs: vec![LegacyTypedParameter { r#type: "type".to_string(), name: "name".to_string() }],
                    outputs: vec![LegacyTypedParameter { r#type: "type".to_string(), name: "name".to_string() }],
                    state_mutability: Some(FunctionStateMutability::View),
                }),
                LegacyContractAbiEntry::Event(LegacyEventAbiEntry {
                    r#type: LegacyEventAbiType::Event,
                    name: "event".to_string(),
                    keys: vec![LegacyTypedParameter { r#type: "type".to_string(), name: "name".to_string() }],
                    data: vec![LegacyTypedParameter { r#type: "type".to_string(), name: "name".to_string() }],
                }),
                LegacyContractAbiEntry::Struct(LegacyStructAbiEntry {
                    r#type: LegacyStructAbiType::Struct,
                    name: "struct".to_string(),
                    size: 1,
                    members: vec![LegacyStructMember {
                        name: "name".to_string(),
                        r#type: "type".to_string(),
                        offset: 1,
                    }],
                }),
            ]),
        };

        let contract_class: ContractClass = legacy_contract_class.clone().into();

        assert_consistent_conversion::<_, StarknetContractClass>(contract_class);
    }

    #[test]
    fn test_sierra_contract_class_conversion() {
        let sierra_contract_class = FlattenedSierraClass {
            sierra_program: vec![Felt::from(1), Felt::from(2), Felt::from(3)],
            contract_class_version: "1.2.3".to_string(),
            entry_points_by_type: EntryPointsByType {
                constructor: vec![SierraEntryPoint { selector: Felt::from(1), function_idx: 1 }],
                external: vec![SierraEntryPoint { selector: Felt::from(2), function_idx: 2 }],
                l1_handler: vec![SierraEntryPoint { selector: Felt::from(3), function_idx: 3 }],
            },
            abi: "abi definition".to_string(),
        };

        let contract_class: ContractClass = sierra_contract_class.into();

        assert_consistent_conversion::<_, StarknetContractClass>(contract_class);
    }
}
