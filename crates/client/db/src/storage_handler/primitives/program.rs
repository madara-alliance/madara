use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use cairo_vm::felt::Felt252;
use cairo_vm::hint_processor::hint_processor_definition::HintReference;
use cairo_vm::serde::deserialize_program::{felt_from_number, ApTracking, BuiltinName, InstructionLocation, Member};
use cairo_vm::types::program::{HintRange, Program};
use cairo_vm::types::relocatable::MaybeRelocatable;
use serde::de::MapAccess;
use serde::{de, Deserialize, Deserializer, Serialize};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProgramWrapper {
    pub shared_program_data: Arc<SharedProgramDataWrapper>,
    pub constants: BTreeMap<String, Felt252>,
    pub builtins: Vec<BuiltinName>,
}

#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub struct SharedProgramDataWrapper {
    pub data: Vec<MaybeRelocatable>,
    pub hints_collection: HintsCollectionWrapper,
    pub main: Option<usize>,
    pub start: Option<usize>,
    pub end: Option<usize>,
    pub error_message_attributes: Vec<AttributeWrapper>,
    pub instruction_locations: Option<BTreeMap<usize, InstructionLocation>>,
    pub identifiers: BTreeMap<String, IdentifierWrapper>,
    pub reference_manager: Vec<HintReference>,
}

#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub struct HintsCollectionWrapper {
    pub hints: Vec<HintParamsWrapper>,
    pub hints_ranges: Vec<HintRange>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct HintParamsWrapper {
    pub code: String,
    pub accessible_scopes: Vec<String>,
    pub flow_tracking_data: FlowTrackingDataWrapper,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct FlowTrackingDataWrapper {
    pub ap_tracking: ApTracking,
    #[serde(deserialize_with = "deserialize_map_to_string_and_usize_btreemap")]
    pub reference_ids: BTreeMap<String, usize>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AttributeWrapper {
    pub name: String,
    pub start_pc: usize,
    pub end_pc: usize,
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flow_tracking_data: Option<FlowTrackingDataWrapper>,
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Clone)]
pub struct IdentifierWrapper {
    pub pc: Option<usize>,
    #[serde(rename(deserialize = "type"))]
    pub type_: Option<String>,
    #[serde(default)]
    #[serde(deserialize_with = "felt_from_number")]
    pub value: Option<Felt252>,

    pub full_name: Option<String>,
    pub members: Option<BTreeMap<String, Member>>,
    pub cairo_type: Option<String>,
}

pub struct ReferenceIdsVisitorWrapper;

impl<'de> de::Visitor<'de> for ReferenceIdsVisitorWrapper {
    type Value = BTreeMap<String, usize>;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a map with string keys and integer values")
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut data: BTreeMap<String, usize> = BTreeMap::new();

        while let Some((key, value)) = map.next_entry::<String, usize>()? {
            data.insert(key, value);
        }

        Ok(data)
    }
}

impl From<&HintsCollectionWrapper> for BTreeMap<usize, Vec<HintParamsWrapper>> {
    fn from(hc: &HintsCollectionWrapper) -> Self {
        let mut hint_map = BTreeMap::new();
        for (i, r) in hc.hints_ranges.iter().enumerate() {
            let Some(r) = r else {
                continue;
            };
            hint_map.insert(i, hc.hints[r.0..r.0 + r.1.get()].to_owned());
        }
        hint_map
    }
}

/// Orders all the HashMaps in the Program structure, replacing them with BTreeMaps
pub fn order_program(program: &Program) -> ProgramWrapper {
    let shared_program_data = &program.shared_program_data;

    let ordered_hints_collection = HintsCollectionWrapper {
        hints: shared_program_data
            .hints_collection
            .hints
            .iter()
            .map(|hint| HintParamsWrapper {
                code: hint.code.clone(),
                accessible_scopes: hint.accessible_scopes.clone(),
                flow_tracking_data: FlowTrackingDataWrapper {
                    ap_tracking: hint.flow_tracking_data.ap_tracking,
                    reference_ids: hint
                        .flow_tracking_data
                        .reference_ids
                        .iter()
                        .map(|(k, v)| (k.clone(), *v))
                        .collect::<BTreeMap<_, _>>(),
                },
            })
            .collect(),
        hints_ranges: shared_program_data.hints_collection.hints_ranges.clone(),
    };

    let ordered_identifiers = shared_program_data
        .identifiers
        .iter()
        .map(|(key, value)| {
            let ordered_members = value
                .members
                .as_ref()
                .map(|members| members.iter().map(|(k, v)| (k.clone(), v.clone())).collect::<BTreeMap<_, _>>());
            (
                key.clone(),
                IdentifierWrapper {
                    pc: value.pc,
                    type_: value.type_.clone(),
                    value: value.value.clone(),
                    full_name: value.full_name.clone(),
                    members: ordered_members,
                    cairo_type: value.cairo_type.clone(),
                },
            )
        })
        .collect::<BTreeMap<_, _>>();

    let ordered_instruction_locations = shared_program_data
        .instruction_locations
        .as_ref()
        .map(|locations| locations.iter().map(|(key, value)| (*key, value.clone())).collect::<BTreeMap<_, _>>());

    let ordered_error_message_attributes = shared_program_data
        .error_message_attributes
        .iter()
        .map(|attr| {
            let ordered_flow_tracking_data =
                attr.flow_tracking_data.as_ref().map(|flow_tracking_data| FlowTrackingDataWrapper {
                    ap_tracking: flow_tracking_data.ap_tracking,
                    reference_ids: flow_tracking_data
                        .reference_ids
                        .iter()
                        .map(|(k, v)| (k.clone(), *v))
                        .collect::<BTreeMap<_, _>>(),
                });
            AttributeWrapper {
                name: attr.name.clone(),
                start_pc: attr.start_pc,
                end_pc: attr.end_pc,
                value: attr.value.clone(),
                flow_tracking_data: ordered_flow_tracking_data,
            }
        })
        .collect::<Vec<_>>();

    let new_shared_program_data = SharedProgramDataWrapper {
        data: shared_program_data.data.clone(),
        hints_collection: ordered_hints_collection,
        main: shared_program_data.main,
        start: shared_program_data.start,
        end: shared_program_data.end,
        error_message_attributes: ordered_error_message_attributes,
        instruction_locations: ordered_instruction_locations,
        identifiers: ordered_identifiers,
        reference_manager: shared_program_data.reference_manager.clone(),
    };

    let ordered_constants =
        program.constants.iter().map(|(key, value)| (key.clone(), value.clone())).collect::<BTreeMap<_, _>>();

    ProgramWrapper {
        shared_program_data: Arc::new(new_shared_program_data),
        constants: ordered_constants,
        builtins: program.builtins.clone(),
    }
}

pub fn deserialize_map_to_string_and_usize_btreemap<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<BTreeMap<String, usize>, D::Error> {
    d.deserialize_map(ReferenceIdsVisitorWrapper)
}
