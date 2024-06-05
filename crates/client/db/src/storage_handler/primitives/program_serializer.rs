use std::collections::BTreeMap;

use cairo_vm::felt::{Felt252, PRIME_STR};
use cairo_vm::serde::deserialize_program::{ApTracking, BuiltinName, InstructionLocation, Member, ValueAddress};
use cairo_vm::types::errors::program_errors::ProgramError;
use cairo_vm::types::relocatable::MaybeRelocatable;
use serde::{Deserialize, Serialize};

use super::program::{AttributeWrapper, FlowTrackingDataWrapper, HintParamsWrapper, IdentifierWrapper, ProgramWrapper};

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub(crate) struct ProgramSerializer {
    pub prime: String,
    pub builtins: Vec<BuiltinName>,
    pub data: Vec<MaybeRelocatable>,
    pub identifiers: BTreeMap<String, IdentifierSerializerWrapper>,
    pub hints: BTreeMap<usize, Vec<HintParamsSerializerWrapper>>,
    pub reference_manager: ReferenceManagerSerializerWrapper,
    pub attributes: Vec<AttributeWrapper>,
    pub debug_info: Option<DebugInfoWrapper>,
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Clone)]
pub(crate) struct IdentifierSerializerWrapper {
    pub pc: Option<usize>,
    pub type_: Option<String>,
    #[serde(default)]
    pub value: Option<Felt252>,

    pub full_name: Option<String>,
    pub members: Option<BTreeMap<String, Member>>,
    pub cairo_type: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct HintParamsSerializerWrapper {
    pub code: String,
    pub accessible_scopes: Vec<String>,
    pub flow_tracking_data: FlowTrackingDataSerializerWrapper,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct FlowTrackingDataSerializerWrapper {
    pub ap_tracking: ApTracking,
    pub reference_ids: BTreeMap<String, usize>,
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Clone, Default)]
pub struct ReferenceManagerSerializerWrapper {
    pub references: Vec<ReferenceSerializerWrapper>,
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Clone)]
pub struct ReferenceSerializerWrapper {
    pub ap_tracking_data: ApTracking,
    pub pc: Option<usize>,
    pub value_address: ValueAddress,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct DebugInfoWrapper {
    pub(crate) instruction_locations: BTreeMap<usize, InstructionLocation>,
}

pub fn serialize_program(program: &ProgramWrapper) -> Result<Vec<u8>, ProgramError> {
    let program_serializer: ProgramSerializer = ProgramSerializer::from(program);
    let bytes: Vec<u8> = serde_json::to_vec(&program_serializer)?;
    Ok(bytes)
}

pub fn prime(program: &ProgramWrapper) -> &str {
    _ = program;
    PRIME_STR
}

impl From<&ProgramWrapper> for ProgramSerializer {
    fn from(program: &ProgramWrapper) -> Self {
        let references = program
            .shared_program_data
            .reference_manager
            .clone()
            .into_iter()
            .map(|r| ReferenceSerializerWrapper {
                value_address: ValueAddress {
                    offset1: r.offset1,
                    offset2: r.offset2,
                    dereference: r.dereference,
                    value_type: r.cairo_type.unwrap_or_default(),
                },
                ap_tracking_data: r.ap_tracking_data.unwrap_or_default(),
                pc: None,
            })
            .collect::<Vec<_>>();

        let mut identifiers = BTreeMap::new();
        for (key, identifier) in program.shared_program_data.identifiers.clone() {
            identifiers.insert(key, identifier.into());
        }

        let mut hints: BTreeMap<usize, Vec<HintParamsSerializerWrapper>> = BTreeMap::new();
        for (key, hint_params_vec) in BTreeMap::from(&program.shared_program_data.hints_collection) {
            let mut new_hints_params = Vec::new();
            for hint_params in hint_params_vec {
                new_hints_params.push(hint_params.clone().into());
            }
            hints.insert(key, new_hints_params);
        }

        ProgramSerializer {
            prime: prime(program).into(),
            builtins: program.builtins.clone(),
            data: program.shared_program_data.data.clone(),
            identifiers,
            hints,
            attributes: program.shared_program_data.error_message_attributes.clone(),
            debug_info: program
                .shared_program_data
                .instruction_locations
                .clone()
                .map(|instruction_locations| DebugInfoWrapper { instruction_locations }),
            reference_manager: ReferenceManagerSerializerWrapper { references },
        }
    }
}

impl From<IdentifierWrapper> for IdentifierSerializerWrapper {
    fn from(identifier_serialer: IdentifierWrapper) -> IdentifierSerializerWrapper {
        IdentifierSerializerWrapper {
            pc: identifier_serialer.pc,
            type_: identifier_serialer.type_,
            value: identifier_serialer.value,
            full_name: identifier_serialer.full_name,
            members: identifier_serialer.members,
            cairo_type: identifier_serialer.cairo_type,
        }
    }
}

impl From<HintParamsWrapper> for HintParamsSerializerWrapper {
    fn from(hint_params_serializer: HintParamsWrapper) -> HintParamsSerializerWrapper {
        HintParamsSerializerWrapper {
            code: hint_params_serializer.code,
            accessible_scopes: hint_params_serializer.accessible_scopes,
            flow_tracking_data: hint_params_serializer.flow_tracking_data.into(),
        }
    }
}

impl From<FlowTrackingDataWrapper> for FlowTrackingDataSerializerWrapper {
    fn from(flow_tracking_data_serialer: FlowTrackingDataWrapper) -> FlowTrackingDataSerializerWrapper {
        FlowTrackingDataSerializerWrapper {
            ap_tracking: flow_tracking_data_serialer.ap_tracking,
            reference_ids: flow_tracking_data_serialer.reference_ids,
        }
    }
}
