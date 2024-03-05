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
        abi.map(|entries| entries.into_iter().map(AbiEntryWrapper::from).collect())
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

    impl From<AbiEntryWrapper> for LegacyContractAbiEntry {
        fn from(abi_entry: AbiEntryWrapper) -> Self {
            match abi_entry {
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
                outputs: abi_function_entry.outputs.iter().map(|v| AbiTypedParameterWrapper::from(v.clone())).collect(),
                state_mutability: abi_function_entry.state_mutability.map(AbiFunctionStateMutabilityWrapper::from),
            }
        }
    }

    impl From<AbiFunctionEntryWrapper> for LegacyFunctionAbiEntry {
        fn from(abi_function_entry: AbiFunctionEntryWrapper) -> Self {
            LegacyFunctionAbiEntry {
                r#type: abi_function_entry.r#type.into(),
                name: abi_function_entry.name,
                inputs: abi_function_entry.inputs.into_iter().map(|v| v.into()).collect(),
                outputs: abi_function_entry.outputs.into_iter().map(|v| v.into()).collect(),
                state_mutability: abi_function_entry.state_mutability.map(|v| v.into()),
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

    impl From<AbiFunctionTypeWrapper> for LegacyFunctionAbiType {
        fn from(abi_function_type: AbiFunctionTypeWrapper) -> Self {
            match abi_function_type {
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

    impl From<AbiFunctionStateMutabilityWrapper> for FunctionStateMutability {
        fn from(abi_func_state_mutability: AbiFunctionStateMutabilityWrapper) -> Self {
            match abi_func_state_mutability {
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
                keys: abi_event_entry.keys.into_iter().map(AbiTypedParameterWrapper::from).collect(),
                data: abi_event_entry.data.into_iter().map(AbiTypedParameterWrapper::from).collect(),
            }
        }
    }

    impl From<AbiEventEntryWrapper> for LegacyEventAbiEntry {
        fn from(abi_event_entry: AbiEventEntryWrapper) -> Self {
            LegacyEventAbiEntry {
                r#type: abi_event_entry.r#type.into(),
                name: abi_event_entry.name,
                keys: abi_event_entry.keys.into_iter().map(|v| v.into()).collect(),
                data: abi_event_entry.data.into_iter().map(|v| v.into()).collect(),
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

    impl From<AbiEventTypeWrapper> for LegacyEventAbiType {
        fn from(abi_event_type: AbiEventTypeWrapper) -> Self {
            match abi_event_type {
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
                members: abi_struct_entry.members.into_iter().map(AbiStructMemberWrapper::from).collect(),
            }
        }
    }

    impl From<AbiStructEntryWrapper> for LegacyStructAbiEntry {
        fn from(abi_struct_entry: AbiStructEntryWrapper) -> Self {
            LegacyStructAbiEntry {
                r#type: abi_struct_entry.r#type.into(),
                name: abi_struct_entry.name,
                size: abi_struct_entry.size,
                members: abi_struct_entry.members.into_iter().map(|v| v.into()).collect(),
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

    impl From<AbiStructTypeWrapper> for LegacyStructAbiType {
        fn from(abi_struct_type: AbiStructTypeWrapper) -> Self {
            match abi_struct_type {
                AbiStructTypeWrapper::Struct => LegacyStructAbiType::Struct,
            }
        }
    }

    impl From<LegacyStructMember> for AbiStructMemberWrapper {
        fn from(abi_struct_member: LegacyStructMember) -> Self {
            Self { name: abi_struct_member.name, r#type: abi_struct_member.r#type, offset: abi_struct_member.offset }
        }
    }

    impl From<AbiStructMemberWrapper> for LegacyStructMember {
        fn from(abi_struct_member: AbiStructMemberWrapper) -> Self {
            LegacyStructMember {
                name: abi_struct_member.name,
                r#type: abi_struct_member.r#type,
                offset: abi_struct_member.offset,
            }
        }
    }

    impl From<LegacyTypedParameter> for AbiTypedParameterWrapper {
        fn from(abi_typed_parameter: LegacyTypedParameter) -> Self {
            Self { name: abi_typed_parameter.name, r#type: abi_typed_parameter.r#type }
        }
    }

    impl From<AbiTypedParameterWrapper> for LegacyTypedParameter {
        fn from(abi_typed_parameter: AbiTypedParameterWrapper) -> Self {
            LegacyTypedParameter { name: abi_typed_parameter.name, r#type: abi_typed_parameter.r#type }
        }
    }
}
