use std::sync::Arc;

use starknet_types_core::felt::Felt;

use crate::{
    CompressedLegacyContractClass, ContractClass, EntryPointsByType, FlattenedSierraClass, FunctionStateMutability,
    LegacyContractAbiEntry, LegacyContractEntryPoint, LegacyEntryPointsByType, LegacyEventAbiEntry, LegacyEventAbiType,
    LegacyFunctionAbiEntry, LegacyFunctionAbiType, LegacyStructAbiEntry, LegacyStructAbiType, LegacyStructMember,
    LegacyTypedParameter, SierraEntryPoint,
};

impl TryFrom<starknet_types_rpc::MaybeDeprecatedContractClass<Felt>> for ContractClass {
    type Error = std::io::Error;

    fn try_from(contract_class: starknet_types_rpc::MaybeDeprecatedContractClass<Felt>) -> Result<Self, Self::Error> {
        match contract_class {
            starknet_types_rpc::MaybeDeprecatedContractClass::ContractClass(flattened_sierra_class) => {
                Ok(ContractClass::Sierra(Arc::new(flattened_sierra_class.into())))
            }
            starknet_types_rpc::MaybeDeprecatedContractClass::Deprecated(compressed_legacy_contract_class) => {
                Ok(ContractClass::Legacy(Arc::new(compressed_legacy_contract_class.try_into()?)))
            }
        }
    }
}

impl From<ContractClass> for starknet_types_rpc::MaybeDeprecatedContractClass<Felt> {
    fn from(contract_class: ContractClass) -> Self {
        match contract_class {
            ContractClass::Sierra(flattened_sierra_class) => {
                starknet_types_rpc::MaybeDeprecatedContractClass::ContractClass(
                    (*flattened_sierra_class).clone().into(),
                )
            }
            ContractClass::Legacy(compressed_legacy_contract_class) => {
                starknet_types_rpc::MaybeDeprecatedContractClass::Deprecated(
                    (*compressed_legacy_contract_class).clone().into(),
                )
            }
        }
    }
}

impl From<starknet_types_rpc::ContractClass<Felt>> for FlattenedSierraClass {
    fn from(flattened_sierra_class: starknet_types_rpc::ContractClass<Felt>) -> Self {
        FlattenedSierraClass {
            sierra_program: flattened_sierra_class.sierra_program,
            contract_class_version: flattened_sierra_class.contract_class_version,
            entry_points_by_type: flattened_sierra_class.entry_points_by_type.into(),
            abi: flattened_sierra_class.abi.unwrap_or("".to_string()),
        }
    }
}

impl From<FlattenedSierraClass> for starknet_types_rpc::ContractClass<Felt> {
    fn from(flattened_sierra_class: FlattenedSierraClass) -> Self {
        starknet_types_rpc::ContractClass {
            sierra_program: flattened_sierra_class.sierra_program,
            contract_class_version: flattened_sierra_class.contract_class_version,
            entry_points_by_type: flattened_sierra_class.entry_points_by_type.into(),
            abi: Some(flattened_sierra_class.abi),
        }
    }
}

impl From<starknet_types_rpc::EntryPointsByType<Felt>> for EntryPointsByType {
    fn from(entry_points_by_type: starknet_types_rpc::EntryPointsByType<Felt>) -> Self {
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

impl From<EntryPointsByType> for starknet_types_rpc::EntryPointsByType<Felt> {
    fn from(entry_points_by_type: EntryPointsByType) -> Self {
        starknet_types_rpc::EntryPointsByType {
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

impl From<starknet_types_rpc::SierraEntryPoint<Felt>> for SierraEntryPoint {
    fn from(sierra_entry_point: starknet_types_rpc::SierraEntryPoint<Felt>) -> Self {
        SierraEntryPoint { selector: sierra_entry_point.selector, function_idx: sierra_entry_point.function_idx }
    }
}

impl From<SierraEntryPoint> for starknet_types_rpc::SierraEntryPoint<Felt> {
    fn from(sierra_entry_point: SierraEntryPoint) -> Self {
        starknet_types_rpc::SierraEntryPoint {
            selector: sierra_entry_point.selector,
            function_idx: sierra_entry_point.function_idx,
        }
    }
}

impl TryFrom<starknet_types_rpc::DeprecatedContractClass<Felt>> for CompressedLegacyContractClass {
    type Error = std::io::Error;

    fn try_from(
        compressed_legacy_contract_class: starknet_types_rpc::DeprecatedContractClass<Felt>,
    ) -> Result<Self, Self::Error> {
        use base64::Engine;

        let decoded_program = base64::engine::general_purpose::STANDARD
            .decode(&compressed_legacy_contract_class.program)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

        Ok(CompressedLegacyContractClass {
            program: decoded_program,
            entry_points_by_type: compressed_legacy_contract_class.entry_points_by_type.into(),
            abi: compressed_legacy_contract_class
                .abi
                .map(|abi| abi.into_iter().map(|legacy_contract_abi_entry| legacy_contract_abi_entry.into()).collect()),
        })
    }
}

impl From<CompressedLegacyContractClass> for starknet_types_rpc::DeprecatedContractClass<Felt> {
    fn from(compressed_legacy_contract_class: CompressedLegacyContractClass) -> Self {
        use base64::Engine;

        let encoded_program =
            base64::engine::general_purpose::STANDARD.encode(&compressed_legacy_contract_class.program);

        starknet_types_rpc::DeprecatedContractClass {
            program: encoded_program,
            entry_points_by_type: compressed_legacy_contract_class.entry_points_by_type.into(),
            abi: compressed_legacy_contract_class
                .abi
                .map(|abi| abi.into_iter().map(|legacy_contract_abi_entry| legacy_contract_abi_entry.into()).collect()),
        }
    }
}

impl From<starknet_types_rpc::DeprecatedEntryPointsByType<Felt>> for LegacyEntryPointsByType {
    fn from(legacy_entry_points_by_type: starknet_types_rpc::DeprecatedEntryPointsByType<Felt>) -> Self {
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

impl From<LegacyEntryPointsByType> for starknet_types_rpc::DeprecatedEntryPointsByType<Felt> {
    fn from(legacy_entry_points_by_type: LegacyEntryPointsByType) -> Self {
        starknet_types_rpc::DeprecatedEntryPointsByType {
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

impl From<starknet_types_rpc::DeprecatedCairoEntryPoint<Felt>> for LegacyContractEntryPoint {
    fn from(legacy_contract_entry_point: starknet_types_rpc::DeprecatedCairoEntryPoint<Felt>) -> Self {
        LegacyContractEntryPoint {
            offset: legacy_contract_entry_point.offset,
            selector: legacy_contract_entry_point.selector,
        }
    }
}

impl From<LegacyContractEntryPoint> for starknet_types_rpc::DeprecatedCairoEntryPoint<Felt> {
    fn from(legacy_contract_entry_point: LegacyContractEntryPoint) -> Self {
        starknet_types_rpc::DeprecatedCairoEntryPoint {
            offset: legacy_contract_entry_point.offset,
            selector: legacy_contract_entry_point.selector,
        }
    }
}

impl From<starknet_types_rpc::ContractAbiEntry> for LegacyContractAbiEntry {
    fn from(legacy_contract_abi_entry: starknet_types_rpc::ContractAbiEntry) -> Self {
        match legacy_contract_abi_entry {
            starknet_types_rpc::ContractAbiEntry::Function(legacy_function_abi_entry) => {
                LegacyContractAbiEntry::Function(legacy_function_abi_entry.into())
            }
            starknet_types_rpc::ContractAbiEntry::Event(legacy_event_abi_entry) => {
                LegacyContractAbiEntry::Event(legacy_event_abi_entry.into())
            }
            starknet_types_rpc::ContractAbiEntry::Struct(legacy_struct_abi_entry) => {
                LegacyContractAbiEntry::Struct(legacy_struct_abi_entry.into())
            }
        }
    }
}

impl From<LegacyContractAbiEntry> for starknet_types_rpc::ContractAbiEntry {
    fn from(legacy_contract_abi_entry: LegacyContractAbiEntry) -> Self {
        match legacy_contract_abi_entry {
            LegacyContractAbiEntry::Function(legacy_function_abi_entry) => {
                starknet_types_rpc::ContractAbiEntry::Function(legacy_function_abi_entry.into())
            }
            LegacyContractAbiEntry::Event(legacy_event_abi_entry) => {
                starknet_types_rpc::ContractAbiEntry::Event(legacy_event_abi_entry.into())
            }
            LegacyContractAbiEntry::Struct(legacy_struct_abi_entry) => {
                starknet_types_rpc::ContractAbiEntry::Struct(legacy_struct_abi_entry.into())
            }
        }
    }
}

impl From<starknet_types_rpc::FunctionAbiEntry> for LegacyFunctionAbiEntry {
    fn from(legacy_function_abi_entry: starknet_types_rpc::FunctionAbiEntry) -> Self {
        LegacyFunctionAbiEntry {
            r#type: legacy_function_abi_entry.ty.into(),
            name: legacy_function_abi_entry.name,
            inputs: legacy_function_abi_entry.inputs.into_iter().map(|abi_entry| abi_entry.into()).collect(),
            outputs: legacy_function_abi_entry.outputs.into_iter().map(|abi_entry| abi_entry.into()).collect(),
            state_mutability: legacy_function_abi_entry
                .state_mutability
                .map(|state_mutability| state_mutability.into()),
        }
    }
}

impl From<LegacyFunctionAbiEntry> for starknet_types_rpc::FunctionAbiEntry {
    fn from(legacy_function_abi_entry: LegacyFunctionAbiEntry) -> Self {
        starknet_types_rpc::FunctionAbiEntry {
            ty: legacy_function_abi_entry.r#type.into(),
            name: legacy_function_abi_entry.name,
            inputs: legacy_function_abi_entry.inputs.into_iter().map(|abi_entry| abi_entry.into()).collect(),
            outputs: legacy_function_abi_entry.outputs.into_iter().map(|abi_entry| abi_entry.into()).collect(),
            state_mutability: legacy_function_abi_entry
                .state_mutability
                .map(|state_mutability| state_mutability.into()),
        }
    }
}

impl From<starknet_types_rpc::EventAbiEntry> for LegacyEventAbiEntry {
    fn from(legacy_event_abi_entry: starknet_types_rpc::EventAbiEntry) -> Self {
        LegacyEventAbiEntry {
            r#type: legacy_event_abi_entry.ty.into(),
            name: legacy_event_abi_entry.name,
            keys: legacy_event_abi_entry.keys.into_iter().map(|abi_entry| abi_entry.into()).collect(),
            data: legacy_event_abi_entry.data.into_iter().map(|abi_entry| abi_entry.into()).collect(),
        }
    }
}

impl From<LegacyEventAbiEntry> for starknet_types_rpc::EventAbiEntry {
    fn from(legacy_event_abi_entry: LegacyEventAbiEntry) -> Self {
        starknet_types_rpc::EventAbiEntry {
            ty: legacy_event_abi_entry.r#type.into(),
            name: legacy_event_abi_entry.name,
            keys: legacy_event_abi_entry.keys.into_iter().map(|abi_entry| abi_entry.into()).collect(),
            data: legacy_event_abi_entry.data.into_iter().map(|abi_entry| abi_entry.into()).collect(),
        }
    }
}

impl From<starknet_types_rpc::StructAbiEntry> for LegacyStructAbiEntry {
    fn from(legacy_struct_abi_entry: starknet_types_rpc::StructAbiEntry) -> Self {
        LegacyStructAbiEntry {
            r#type: legacy_struct_abi_entry.ty.into(),
            name: legacy_struct_abi_entry.name,
            size: legacy_struct_abi_entry.size,
            members: legacy_struct_abi_entry.members.into_iter().map(|member| member.into()).collect(),
        }
    }
}

impl From<LegacyStructAbiEntry> for starknet_types_rpc::StructAbiEntry {
    fn from(legacy_struct_abi_entry: LegacyStructAbiEntry) -> Self {
        starknet_types_rpc::StructAbiEntry {
            ty: legacy_struct_abi_entry.r#type.into(),
            name: legacy_struct_abi_entry.name,
            size: legacy_struct_abi_entry.size,
            members: legacy_struct_abi_entry.members.into_iter().map(|member| member.into()).collect(),
        }
    }
}

impl From<starknet_types_rpc::StructMember> for LegacyStructMember {
    fn from(legacy_struct_member: starknet_types_rpc::StructMember) -> Self {
        LegacyStructMember {
            name: legacy_struct_member.typed_parameter.name,
            r#type: legacy_struct_member.typed_parameter.ty,
            offset: legacy_struct_member.offset,
        }
    }
}

impl From<LegacyStructMember> for starknet_types_rpc::StructMember {
    fn from(legacy_struct_member: LegacyStructMember) -> Self {
        starknet_types_rpc::StructMember {
            typed_parameter: starknet_types_rpc::TypedParameter {
                name: legacy_struct_member.name,
                ty: legacy_struct_member.r#type,
            },
            offset: legacy_struct_member.offset,
        }
    }
}

impl From<starknet_types_rpc::TypedParameter> for LegacyTypedParameter {
    fn from(legacy_typed_parameter: starknet_types_rpc::TypedParameter) -> Self {
        LegacyTypedParameter { r#type: legacy_typed_parameter.ty, name: legacy_typed_parameter.name }
    }
}

impl From<LegacyTypedParameter> for starknet_types_rpc::TypedParameter {
    fn from(legacy_typed_parameter: LegacyTypedParameter) -> Self {
        starknet_types_rpc::TypedParameter { ty: legacy_typed_parameter.r#type, name: legacy_typed_parameter.name }
    }
}

impl From<starknet_types_rpc::FunctionAbiType> for LegacyFunctionAbiType {
    fn from(legacy_function_abi_type: starknet_types_rpc::FunctionAbiType) -> Self {
        match legacy_function_abi_type {
            starknet_types_rpc::FunctionAbiType::Function => LegacyFunctionAbiType::Function,
            starknet_types_rpc::FunctionAbiType::L1Handler => LegacyFunctionAbiType::L1Handler,
            starknet_types_rpc::FunctionAbiType::Constructor => LegacyFunctionAbiType::Constructor,
        }
    }
}

impl From<LegacyFunctionAbiType> for starknet_types_rpc::FunctionAbiType {
    fn from(legacy_function_abi_type: LegacyFunctionAbiType) -> Self {
        match legacy_function_abi_type {
            LegacyFunctionAbiType::Function => starknet_types_rpc::FunctionAbiType::Function,
            LegacyFunctionAbiType::L1Handler => starknet_types_rpc::FunctionAbiType::L1Handler,
            LegacyFunctionAbiType::Constructor => starknet_types_rpc::FunctionAbiType::Constructor,
        }
    }
}

impl From<starknet_types_rpc::EventAbiType> for LegacyEventAbiType {
    fn from(_: starknet_types_rpc::EventAbiType) -> Self {
        LegacyEventAbiType::Event
    }
}

impl From<LegacyEventAbiType> for starknet_types_rpc::EventAbiType {
    fn from(legacy_event_abi_type: LegacyEventAbiType) -> Self {
        match legacy_event_abi_type {
            LegacyEventAbiType::Event => "event".to_string(),
        }
    }
}

impl From<starknet_types_rpc::StructAbiType> for LegacyStructAbiType {
    fn from(_: starknet_types_rpc::StructAbiType) -> Self {
        LegacyStructAbiType::Struct
    }
}

impl From<LegacyStructAbiType> for starknet_types_rpc::StructAbiType {
    fn from(legacy_struct_abi_type: LegacyStructAbiType) -> Self {
        match legacy_struct_abi_type {
            LegacyStructAbiType::Struct => "struct".to_string(),
        }
    }
}

impl From<starknet_types_rpc::FunctionStateMutability> for FunctionStateMutability {
    fn from(_: starknet_types_rpc::FunctionStateMutability) -> Self {
        FunctionStateMutability::View
    }
}

impl From<FunctionStateMutability> for starknet_types_rpc::FunctionStateMutability {
    fn from(function_state_mutability: FunctionStateMutability) -> Self {
        match function_state_mutability {
            FunctionStateMutability::View => "view".to_string(),
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
    use starknet_types_core::felt::Felt;
    use starknet_types_rpc::MaybeDeprecatedContractClass as StarknetContractClass;

    #[test]
    fn test_legacy_contract_class_conversion() {
        let legacy_contract_class = CompressedLegacyContractClass {
            program: "program".as_bytes().to_vec(),
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

        assert_consistent_conversion::<_, StarknetContractClass<Felt>>(contract_class);
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

        assert_consistent_conversion::<_, StarknetContractClass<Felt>>(contract_class);
    }
}
