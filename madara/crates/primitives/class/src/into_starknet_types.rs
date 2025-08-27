use std::sync::Arc;

use crate::{
    CompressedLegacyContractClass, ContractClass, EntryPointsByType, FlattenedSierraClass, FunctionStateMutability,
    LegacyContractAbiEntry, LegacyContractEntryPoint, LegacyEntryPointsByType, LegacyEventAbiEntry, LegacyEventAbiType,
    LegacyFunctionAbiEntry, LegacyFunctionAbiType, LegacyStructAbiEntry, LegacyStructAbiType, LegacyStructMember,
    LegacyTypedParameter, SierraEntryPoint,
};

impl TryFrom<mp_rpc::v0_7_1::MaybeDeprecatedContractClass> for ContractClass {
    type Error = std::io::Error;

    fn try_from(contract_class: mp_rpc::v0_7_1::MaybeDeprecatedContractClass) -> Result<Self, Self::Error> {
        match contract_class {
            mp_rpc::v0_7_1::MaybeDeprecatedContractClass::ContractClass(flattened_sierra_class) => {
                Ok(ContractClass::Sierra(Arc::new(flattened_sierra_class.into())))
            }
            mp_rpc::v0_7_1::MaybeDeprecatedContractClass::Deprecated(compressed_legacy_contract_class) => {
                Ok(ContractClass::Legacy(Arc::new(compressed_legacy_contract_class.try_into()?)))
            }
        }
    }
}

impl From<ContractClass> for mp_rpc::v0_7_1::MaybeDeprecatedContractClass {
    fn from(contract_class: ContractClass) -> Self {
        match contract_class {
            ContractClass::Sierra(flattened_sierra_class) => {
                mp_rpc::v0_7_1::MaybeDeprecatedContractClass::ContractClass((*flattened_sierra_class).clone().into())
            }
            ContractClass::Legacy(compressed_legacy_contract_class) => {
                mp_rpc::v0_7_1::MaybeDeprecatedContractClass::Deprecated(
                    (*compressed_legacy_contract_class).clone().into(),
                )
            }
        }
    }
}

impl From<mp_rpc::v0_7_1::ContractClass> for FlattenedSierraClass {
    fn from(flattened_sierra_class: mp_rpc::v0_7_1::ContractClass) -> Self {
        FlattenedSierraClass {
            sierra_program: flattened_sierra_class.sierra_program,
            contract_class_version: flattened_sierra_class.contract_class_version,
            entry_points_by_type: flattened_sierra_class.entry_points_by_type.into(),
            abi: flattened_sierra_class.abi.unwrap_or("".to_string()),
        }
    }
}

impl From<FlattenedSierraClass> for mp_rpc::v0_7_1::ContractClass {
    fn from(flattened_sierra_class: FlattenedSierraClass) -> Self {
        mp_rpc::v0_7_1::ContractClass {
            sierra_program: flattened_sierra_class.sierra_program,
            contract_class_version: flattened_sierra_class.contract_class_version,
            entry_points_by_type: flattened_sierra_class.entry_points_by_type.into(),
            abi: Some(flattened_sierra_class.abi),
        }
    }
}

impl From<mp_rpc::v0_7_1::EntryPointsByType> for EntryPointsByType {
    fn from(entry_points_by_type: mp_rpc::v0_7_1::EntryPointsByType) -> Self {
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

impl From<EntryPointsByType> for mp_rpc::v0_7_1::EntryPointsByType {
    fn from(entry_points_by_type: EntryPointsByType) -> Self {
        mp_rpc::v0_7_1::EntryPointsByType {
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

impl From<mp_rpc::v0_7_1::SierraEntryPoint> for SierraEntryPoint {
    fn from(sierra_entry_point: mp_rpc::v0_7_1::SierraEntryPoint) -> Self {
        SierraEntryPoint { selector: sierra_entry_point.selector, function_idx: sierra_entry_point.function_idx }
    }
}

impl From<SierraEntryPoint> for mp_rpc::v0_7_1::SierraEntryPoint {
    fn from(sierra_entry_point: SierraEntryPoint) -> Self {
        mp_rpc::v0_7_1::SierraEntryPoint {
            selector: sierra_entry_point.selector,
            function_idx: sierra_entry_point.function_idx,
        }
    }
}

impl TryFrom<mp_rpc::v0_7_1::DeprecatedContractClass> for CompressedLegacyContractClass {
    type Error = std::io::Error;

    fn try_from(
        compressed_legacy_contract_class: mp_rpc::v0_7_1::DeprecatedContractClass,
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

impl From<CompressedLegacyContractClass> for mp_rpc::v0_7_1::DeprecatedContractClass {
    fn from(compressed_legacy_contract_class: CompressedLegacyContractClass) -> Self {
        use base64::Engine;

        let encoded_program =
            base64::engine::general_purpose::STANDARD.encode(&compressed_legacy_contract_class.program);

        mp_rpc::v0_7_1::DeprecatedContractClass {
            program: encoded_program,
            entry_points_by_type: compressed_legacy_contract_class.entry_points_by_type.into(),
            abi: compressed_legacy_contract_class
                .abi
                .map(|abi| abi.into_iter().map(|legacy_contract_abi_entry| legacy_contract_abi_entry.into()).collect()),
        }
    }
}

impl From<mp_rpc::v0_7_1::DeprecatedEntryPointsByType> for LegacyEntryPointsByType {
    fn from(legacy_entry_points_by_type: mp_rpc::v0_7_1::DeprecatedEntryPointsByType) -> Self {
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

impl From<LegacyEntryPointsByType> for mp_rpc::v0_7_1::DeprecatedEntryPointsByType {
    fn from(legacy_entry_points_by_type: LegacyEntryPointsByType) -> Self {
        mp_rpc::v0_7_1::DeprecatedEntryPointsByType {
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

impl From<mp_rpc::v0_7_1::DeprecatedCairoEntryPoint> for LegacyContractEntryPoint {
    fn from(legacy_contract_entry_point: mp_rpc::v0_7_1::DeprecatedCairoEntryPoint) -> Self {
        LegacyContractEntryPoint {
            offset: legacy_contract_entry_point.offset,
            selector: legacy_contract_entry_point.selector,
        }
    }
}

impl From<LegacyContractEntryPoint> for mp_rpc::v0_7_1::DeprecatedCairoEntryPoint {
    fn from(legacy_contract_entry_point: LegacyContractEntryPoint) -> Self {
        mp_rpc::v0_7_1::DeprecatedCairoEntryPoint {
            offset: legacy_contract_entry_point.offset,
            selector: legacy_contract_entry_point.selector,
        }
    }
}

impl From<mp_rpc::v0_7_1::ContractAbiEntry> for LegacyContractAbiEntry {
    fn from(legacy_contract_abi_entry: mp_rpc::v0_7_1::ContractAbiEntry) -> Self {
        match legacy_contract_abi_entry {
            mp_rpc::v0_7_1::ContractAbiEntry::Function(legacy_function_abi_entry) => {
                LegacyContractAbiEntry::Function(legacy_function_abi_entry.into())
            }
            mp_rpc::v0_7_1::ContractAbiEntry::Event(legacy_event_abi_entry) => {
                LegacyContractAbiEntry::Event(legacy_event_abi_entry.into())
            }
            mp_rpc::v0_7_1::ContractAbiEntry::Struct(legacy_struct_abi_entry) => {
                LegacyContractAbiEntry::Struct(legacy_struct_abi_entry.into())
            }
        }
    }
}

impl From<LegacyContractAbiEntry> for mp_rpc::v0_7_1::ContractAbiEntry {
    fn from(legacy_contract_abi_entry: LegacyContractAbiEntry) -> Self {
        match legacy_contract_abi_entry {
            LegacyContractAbiEntry::Function(legacy_function_abi_entry) => {
                mp_rpc::v0_7_1::ContractAbiEntry::Function(legacy_function_abi_entry.into())
            }
            LegacyContractAbiEntry::Event(legacy_event_abi_entry) => {
                mp_rpc::v0_7_1::ContractAbiEntry::Event(legacy_event_abi_entry.into())
            }
            LegacyContractAbiEntry::Struct(legacy_struct_abi_entry) => {
                mp_rpc::v0_7_1::ContractAbiEntry::Struct(legacy_struct_abi_entry.into())
            }
        }
    }
}

impl From<mp_rpc::v0_7_1::FunctionAbiEntry> for LegacyFunctionAbiEntry {
    fn from(legacy_function_abi_entry: mp_rpc::v0_7_1::FunctionAbiEntry) -> Self {
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

impl From<LegacyFunctionAbiEntry> for mp_rpc::v0_7_1::FunctionAbiEntry {
    fn from(legacy_function_abi_entry: LegacyFunctionAbiEntry) -> Self {
        mp_rpc::v0_7_1::FunctionAbiEntry {
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

impl From<mp_rpc::v0_7_1::EventAbiEntry> for LegacyEventAbiEntry {
    fn from(legacy_event_abi_entry: mp_rpc::v0_7_1::EventAbiEntry) -> Self {
        LegacyEventAbiEntry {
            r#type: legacy_event_abi_entry.ty.into(),
            name: legacy_event_abi_entry.name,
            keys: legacy_event_abi_entry.keys.into_iter().map(|abi_entry| abi_entry.into()).collect(),
            data: legacy_event_abi_entry.data.into_iter().map(|abi_entry| abi_entry.into()).collect(),
        }
    }
}

impl From<LegacyEventAbiEntry> for mp_rpc::v0_7_1::EventAbiEntry {
    fn from(legacy_event_abi_entry: LegacyEventAbiEntry) -> Self {
        mp_rpc::v0_7_1::EventAbiEntry {
            ty: legacy_event_abi_entry.r#type.into(),
            name: legacy_event_abi_entry.name,
            keys: legacy_event_abi_entry.keys.into_iter().map(|abi_entry| abi_entry.into()).collect(),
            data: legacy_event_abi_entry.data.into_iter().map(|abi_entry| abi_entry.into()).collect(),
        }
    }
}

impl From<mp_rpc::v0_7_1::StructAbiEntry> for LegacyStructAbiEntry {
    fn from(legacy_struct_abi_entry: mp_rpc::v0_7_1::StructAbiEntry) -> Self {
        LegacyStructAbiEntry {
            r#type: legacy_struct_abi_entry.ty.into(),
            name: legacy_struct_abi_entry.name,
            size: legacy_struct_abi_entry.size,
            members: legacy_struct_abi_entry.members.into_iter().map(|member| member.into()).collect(),
        }
    }
}

impl From<LegacyStructAbiEntry> for mp_rpc::v0_7_1::StructAbiEntry {
    fn from(legacy_struct_abi_entry: LegacyStructAbiEntry) -> Self {
        mp_rpc::v0_7_1::StructAbiEntry {
            ty: legacy_struct_abi_entry.r#type.into(),
            name: legacy_struct_abi_entry.name,
            size: legacy_struct_abi_entry.size,
            members: legacy_struct_abi_entry.members.into_iter().map(|member| member.into()).collect(),
        }
    }
}

impl From<mp_rpc::v0_7_1::StructMember> for LegacyStructMember {
    fn from(legacy_struct_member: mp_rpc::v0_7_1::StructMember) -> Self {
        LegacyStructMember {
            name: legacy_struct_member.typed_parameter.name,
            r#type: legacy_struct_member.typed_parameter.ty,
            offset: legacy_struct_member.offset,
        }
    }
}

impl From<LegacyStructMember> for mp_rpc::v0_7_1::StructMember {
    fn from(legacy_struct_member: LegacyStructMember) -> Self {
        mp_rpc::v0_7_1::StructMember {
            typed_parameter: mp_rpc::v0_7_1::TypedParameter {
                name: legacy_struct_member.name,
                ty: legacy_struct_member.r#type,
            },
            offset: legacy_struct_member.offset,
        }
    }
}

impl From<mp_rpc::v0_7_1::TypedParameter> for LegacyTypedParameter {
    fn from(legacy_typed_parameter: mp_rpc::v0_7_1::TypedParameter) -> Self {
        LegacyTypedParameter { r#type: legacy_typed_parameter.ty, name: legacy_typed_parameter.name }
    }
}

impl From<LegacyTypedParameter> for mp_rpc::v0_7_1::TypedParameter {
    fn from(legacy_typed_parameter: LegacyTypedParameter) -> Self {
        mp_rpc::v0_7_1::TypedParameter { ty: legacy_typed_parameter.r#type, name: legacy_typed_parameter.name }
    }
}

impl From<mp_rpc::v0_7_1::FunctionAbiType> for LegacyFunctionAbiType {
    fn from(legacy_function_abi_type: mp_rpc::v0_7_1::FunctionAbiType) -> Self {
        match legacy_function_abi_type {
            mp_rpc::v0_7_1::FunctionAbiType::Function => LegacyFunctionAbiType::Function,
            mp_rpc::v0_7_1::FunctionAbiType::L1Handler => LegacyFunctionAbiType::L1Handler,
            mp_rpc::v0_7_1::FunctionAbiType::Constructor => LegacyFunctionAbiType::Constructor,
        }
    }
}

impl From<LegacyFunctionAbiType> for mp_rpc::v0_7_1::FunctionAbiType {
    fn from(legacy_function_abi_type: LegacyFunctionAbiType) -> Self {
        match legacy_function_abi_type {
            LegacyFunctionAbiType::Function => mp_rpc::v0_7_1::FunctionAbiType::Function,
            LegacyFunctionAbiType::L1Handler => mp_rpc::v0_7_1::FunctionAbiType::L1Handler,
            LegacyFunctionAbiType::Constructor => mp_rpc::v0_7_1::FunctionAbiType::Constructor,
        }
    }
}

impl From<mp_rpc::v0_7_1::EventAbiType> for LegacyEventAbiType {
    fn from(_: mp_rpc::v0_7_1::EventAbiType) -> Self {
        LegacyEventAbiType::Event
    }
}

impl From<LegacyEventAbiType> for mp_rpc::v0_7_1::EventAbiType {
    fn from(legacy_event_abi_type: LegacyEventAbiType) -> Self {
        match legacy_event_abi_type {
            LegacyEventAbiType::Event => "event".to_string(),
        }
    }
}

impl From<mp_rpc::v0_7_1::StructAbiType> for LegacyStructAbiType {
    fn from(_: mp_rpc::v0_7_1::StructAbiType) -> Self {
        LegacyStructAbiType::Struct
    }
}

impl From<LegacyStructAbiType> for mp_rpc::v0_7_1::StructAbiType {
    fn from(legacy_struct_abi_type: LegacyStructAbiType) -> Self {
        match legacy_struct_abi_type {
            LegacyStructAbiType::Struct => "struct".to_string(),
        }
    }
}

impl From<mp_rpc::v0_7_1::FunctionStateMutability> for FunctionStateMutability {
    fn from(_: mp_rpc::v0_7_1::FunctionStateMutability) -> Self {
        FunctionStateMutability::View
    }
}

impl From<FunctionStateMutability> for mp_rpc::v0_7_1::FunctionStateMutability {
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
    use mp_rpc::v0_7_1::MaybeDeprecatedContractClass as StarknetContractClass;
    use starknet_types_core::felt::Felt;

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
