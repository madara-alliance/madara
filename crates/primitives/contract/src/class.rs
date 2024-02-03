#![cfg_attr(not(feature = "std"), no_std)]

use blockifier::execution::contract_class::ContractClass as ContractClassBlockifier;
#[cfg(feature = "parity-scale-codec")]
use parity_scale_codec::{Decode, Encode};
use starknet_api::api_core::ClassHash;

use crate::ContractAbi;

#[doc(hidden)]
pub extern crate alloc;
use alloc::vec::Vec;

#[derive(Debug)]
#[cfg_attr(feature = "parity-scale-codec", derive(Encode, Decode))]
pub struct ClassUpdateWrapper(pub Vec<ContractClassData>);

#[derive(Debug)]
#[cfg_attr(feature = "parity-scale-codec", derive(Encode, Decode))]
pub struct ContractClassData {
    pub hash: ClassHash,
    pub contract_class: ContractClassWrapper,
}

#[derive(Debug)]
#[cfg_attr(feature = "parity-scale-codec", derive(Encode, Decode))]
pub struct ContractClassWrapper {
    pub contract: ContractClassBlockifier,
    pub abi: ContractAbi,
}

#[cfg(feature = "std")]
pub mod convert {
    use mp_convert::contract::{from_rpc_contract_class, to_contract_class_cairo, to_contract_class_sierra};
    use starknet_core::types::{
        ContractClass as ContractClassCore, FunctionStateMutability, LegacyContractAbiEntry, LegacyEventAbiEntry,
        LegacyEventAbiType, LegacyFunctionAbiEntry, LegacyFunctionAbiType, LegacyStructAbiEntry, LegacyStructAbiType,
        LegacyStructMember, LegacyTypedParameter,
    };

    use super::*;
    use crate::{
        AbiEntryWrapper, AbiEventEntryWrapper, AbiEventTypeWrapper, AbiFunctionEntryWrapper,
        AbiFunctionStateMutabilityWrapper, AbiFunctionTypeWrapper, AbiStructEntryWrapper, AbiStructMemberWrapper,
        AbiStructTypeWrapper, AbiTypedParameterWrapper, ContractAbi,
    };

    // Wrapper Class conversion

    impl TryFrom<ContractClassCore> for ContractClassWrapper {
        type Error = anyhow::Error;

        fn try_from(contract_class: ContractClassCore) -> Result<Self, Self::Error> {
            Ok(Self {
                contract: from_rpc_contract_class(&contract_class)?,
                abi: match contract_class {
                    ContractClassCore::Sierra(class_sierra) => ContractAbi::Sierra(class_sierra.abi),
                    ContractClassCore::Legacy(class_cairo) => {
                        ContractAbi::Cairo(from_rpc_contract_abi(class_cairo.abi))
                    }
                },
            })
        }
    }

    impl TryInto<ContractClassCore> for ContractClassWrapper {
        type Error = anyhow::Error;

        fn try_into(self) -> Result<ContractClassCore, Self::Error> {
            match self.abi {
                ContractAbi::Sierra(abi_sierra) => {
                    if let ContractClassBlockifier::V1(contract) = self.contract {
                        to_contract_class_sierra(&contract, abi_sierra)
                    } else {
                        unreachable!("should not mix Sierra abi with Cairo contract")
                    }
                }
                ContractAbi::Cairo(abi_cairo) => {
                    if let ContractClassBlockifier::V0(contract) = self.contract {
                        to_contract_class_cairo(&contract, to_rpc_contract_abi(abi_cairo))
                    } else {
                        unreachable!("should not mix Cairo abi with Sierra contract")
                    }
                }
            }
        }
    }

    fn to_rpc_contract_abi(abi: Option<Vec<AbiEntryWrapper>>) -> Option<Vec<LegacyContractAbiEntry>> {
        abi.map(|entries| entries.into_iter().map(|v| v.into()).collect())
    }

    fn from_rpc_contract_abi(abi: Option<Vec<LegacyContractAbiEntry>>) -> Option<Vec<AbiEntryWrapper>> {
        abi.map(|entries| entries.into_iter().map(|v| AbiEntryWrapper::from(v)).collect())
    }

    impl From<LegacyContractAbiEntry> for AbiEntryWrapper {
        fn from(abi_entry: LegacyContractAbiEntry) -> Self {
            match abi_entry {
                LegacyContractAbiEntry::Function(abi_function) => {
                    AbiEntryWrapper::Function(AbiFunctionEntryWrapper::from(abi_function))
                }
                LegacyContractAbiEntry::Event(abi_event) => {
                    AbiEntryWrapper::Event(AbiEventEntryWrapper::from(abi_event))
                }
                LegacyContractAbiEntry::Struct(abi_struct) => {
                    AbiEntryWrapper::Struct(AbiStructEntryWrapper::from(abi_struct))
                }
            }
        }
    }

    impl Into<LegacyContractAbiEntry> for AbiEntryWrapper {
        fn into(self) -> LegacyContractAbiEntry {
            match self {
                AbiEntryWrapper::Function(abi_function) => LegacyContractAbiEntry::Function(abi_function.into()),
                AbiEntryWrapper::Event(abi_event) => LegacyContractAbiEntry::Event(abi_event.into()),
                AbiEntryWrapper::Struct(abi_struct) => LegacyContractAbiEntry::Struct(abi_struct.into()),
            }
        }
    }

    // Function ABI Entry conversion

    impl From<LegacyFunctionAbiEntry> for AbiFunctionEntryWrapper {
        fn from(abi_function_entry: LegacyFunctionAbiEntry) -> Self {
            Self {
                r#type: AbiFunctionTypeWrapper::from(abi_function_entry.r#type),
                name: abi_function_entry.name,
                inputs: abi_function_entry.inputs.iter().map(|v| AbiTypedParameterWrapper::from(v.clone())).collect(),
                outputs: abi_function_entry.inputs.iter().map(|v| AbiTypedParameterWrapper::from(v.clone())).collect(),
                state_mutability: abi_function_entry
                    .state_mutability
                    .map(|v| AbiFunctionStateMutabilityWrapper::from(v)),
            }
        }
    }

    impl Into<LegacyFunctionAbiEntry> for AbiFunctionEntryWrapper {
        fn into(self) -> LegacyFunctionAbiEntry {
            LegacyFunctionAbiEntry {
                r#type: self.r#type.into(),
                name: self.name,
                inputs: self.inputs.into_iter().map(|v| v.into()).collect(),
                outputs: self.outputs.into_iter().map(|v| v.into()).collect(),
                state_mutability: self.state_mutability.map(|v| v.into()),
            }
        }
    }

    impl From<LegacyFunctionAbiType> for AbiFunctionTypeWrapper {
        fn from(abi_func_type: LegacyFunctionAbiType) -> Self {
            match abi_func_type {
                LegacyFunctionAbiType::Function => AbiFunctionTypeWrapper::Function,
                LegacyFunctionAbiType::L1Handler => AbiFunctionTypeWrapper::L1handler,
                LegacyFunctionAbiType::Constructor => AbiFunctionTypeWrapper::Constructor,
            }
        }
    }

    impl Into<LegacyFunctionAbiType> for AbiFunctionTypeWrapper {
        fn into(self) -> LegacyFunctionAbiType {
            match self {
                AbiFunctionTypeWrapper::Function => LegacyFunctionAbiType::Function,
                AbiFunctionTypeWrapper::L1handler => LegacyFunctionAbiType::L1Handler,
                AbiFunctionTypeWrapper::Constructor => LegacyFunctionAbiType::Constructor,
            }
        }
    }

    impl From<FunctionStateMutability> for AbiFunctionStateMutabilityWrapper {
        fn from(abi_func_state_mutability: FunctionStateMutability) -> Self {
            match abi_func_state_mutability {
                FunctionStateMutability::View => AbiFunctionStateMutabilityWrapper::View,
            }
        }
    }

    impl Into<FunctionStateMutability> for AbiFunctionStateMutabilityWrapper {
        fn into(self) -> FunctionStateMutability {
            match self {
                AbiFunctionStateMutabilityWrapper::View => FunctionStateMutability::View,
            }
        }
    }

    // Event ABI Entry conversion

    impl From<LegacyEventAbiEntry> for AbiEventEntryWrapper {
        fn from(abi_event_entry: LegacyEventAbiEntry) -> Self {
            Self {
                r#type: AbiEventTypeWrapper::from(abi_event_entry.r#type),
                name: abi_event_entry.name,
                keys: abi_event_entry.keys.into_iter().map(|v| AbiTypedParameterWrapper::from(v)).collect(),
                data: abi_event_entry.data.into_iter().map(|v| AbiTypedParameterWrapper::from(v)).collect(),
            }
        }
    }

    impl Into<LegacyEventAbiEntry> for AbiEventEntryWrapper {
        fn into(self) -> LegacyEventAbiEntry {
            LegacyEventAbiEntry {
                r#type: self.r#type.into(),
                name: self.name,
                keys: self.keys.into_iter().map(|v| v.into()).collect(),
                data: self.data.into_iter().map(|v| v.into()).collect(),
            }
        }
    }

    impl From<LegacyEventAbiType> for AbiEventTypeWrapper {
        fn from(abi_entry_type: LegacyEventAbiType) -> Self {
            match abi_entry_type {
                LegacyEventAbiType::Event => AbiEventTypeWrapper::Event,
            }
        }
    }

    impl Into<LegacyEventAbiType> for AbiEventTypeWrapper {
        fn into(self) -> LegacyEventAbiType {
            match self {
                AbiEventTypeWrapper::Event => LegacyEventAbiType::Event,
            }
        }
    }

    // Struct ABI Entry conversion

    impl From<LegacyStructAbiEntry> for AbiStructEntryWrapper {
        fn from(abi_struct_entry: LegacyStructAbiEntry) -> Self {
            Self {
                r#type: AbiStructTypeWrapper::from(abi_struct_entry.r#type),
                name: abi_struct_entry.name,
                size: abi_struct_entry.size,
                members: abi_struct_entry.members.into_iter().map(|v| AbiStructMemberWrapper::from(v)).collect(),
            }
        }
    }

    impl Into<LegacyStructAbiEntry> for AbiStructEntryWrapper {
        fn into(self) -> LegacyStructAbiEntry {
            LegacyStructAbiEntry {
                r#type: self.r#type.into(),
                name: self.name,
                size: self.size,
                members: self.members.into_iter().map(|v| v.into()).collect(),
            }
        }
    }

    impl From<LegacyStructAbiType> for AbiStructTypeWrapper {
        fn from(abi_struct_type: LegacyStructAbiType) -> Self {
            match abi_struct_type {
                LegacyStructAbiType::Struct => AbiStructTypeWrapper::Struct,
            }
        }
    }

    impl Into<LegacyStructAbiType> for AbiStructTypeWrapper {
        fn into(self) -> LegacyStructAbiType {
            match self {
                AbiStructTypeWrapper::Struct => LegacyStructAbiType::Struct,
            }
        }
    }

    impl From<LegacyStructMember> for AbiStructMemberWrapper {
        fn from(abi_struct_member: LegacyStructMember) -> Self {
            Self { name: abi_struct_member.name, r#type: abi_struct_member.r#type, offset: abi_struct_member.offset }
        }
    }

    impl Into<LegacyStructMember> for AbiStructMemberWrapper {
        fn into(self) -> LegacyStructMember {
            LegacyStructMember { name: self.name, r#type: self.r#type, offset: self.offset }
        }
    }

    impl From<LegacyTypedParameter> for AbiTypedParameterWrapper {
        fn from(abi_typed_parameter: LegacyTypedParameter) -> Self {
            Self { name: abi_typed_parameter.name, r#type: abi_typed_parameter.r#type }
        }
    }

    impl Into<LegacyTypedParameter> for AbiTypedParameterWrapper {
        fn into(self) -> LegacyTypedParameter {
            LegacyTypedParameter { name: self.name, r#type: self.r#type }
        }
    }
}
