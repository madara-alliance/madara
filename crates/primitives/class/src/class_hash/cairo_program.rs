// Credit: Pathfinder

use std::{borrow::Cow, collections::BTreeMap};

use anyhow::Context;

// Those fields' content are only wrapped to easely implement `AppendToHashChain` on them
#[derive(serde::Deserialize, serde::Serialize)]
#[serde(transparent)]
pub struct Builtins<'a>(pub Vec<Cow<'a, str>>);
#[derive(serde::Deserialize, serde::Serialize)]
#[serde(transparent)]
pub struct Data<'a>(pub Vec<Cow<'a, str>>);

// For performance reasons it is only partially deserialized from the original json
//
// It's important that this is ordered alphabetically because the fields need to
// be in alphabetical order when serialized for the keccak hashed representation.
#[derive(serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct CairoProgram<'a> {
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub attributes: Vec<serde_json::Value>,

    #[serde(borrow)]
    pub builtins: Builtins<'a>,

    // Added in Starknet 0.10, so we have to handle this not being present.
    #[serde(borrow, skip_serializing_if = "Option::is_none")]
    pub compiler_version: Option<Cow<'a, str>>,

    #[serde(borrow)]
    pub data: Data<'a>,

    #[serde(borrow)]
    pub debug_info: Option<&'a serde_json::value::RawValue>,

    // Important that this is ordered by the numeric keys, not lexicographically
    pub hints: BTreeMap<u64, Vec<serde_json::Value>>,

    pub identifiers: serde_json::Value,

    #[serde(borrow)]
    pub main_scope: Cow<'a, str>,

    // Unlike most other integers, this one is hex string. We don't need to interpret it,
    // it just needs to be part of the hashed output.
    #[serde(borrow)]
    pub prime: Cow<'a, str>,

    pub reference_manager: serde_json::Value,
}

impl<'a> CairoProgram<'a> {
    pub fn prepare(&mut self) -> anyhow::Result<()> {
        // the other modification is handled by skipping if the attributes vec is empty
        self.debug_info = None;

        // Cairo 0.8 added "accessible_scopes" and "flow_tracking_data" attribute
        // fields, which were not present in older contracts. They present as null /
        // empty for older contracts and should not be included in the hash
        // calculation in these cases.
        //
        // We therefore check and remove them from the definition before calculating the
        // hash.
        self.attributes.iter_mut().try_for_each(|attr| -> anyhow::Result<()> {
            let vals = attr.as_object_mut().context("Program attribute was not an object")?;

            match vals.get_mut("accessible_scopes") {
                Some(serde_json::Value::Array(array)) => {
                    if array.is_empty() {
                        vals.remove("accessible_scopes");
                    }
                }
                Some(_other) => {
                    anyhow::bail!(r#"A program's attribute["accessible_scopes"] was not an array type."#);
                }
                None => {}
            }
            // We don't know what this type is supposed to be, but if its missing it is
            // null.
            if let Some(serde_json::Value::Null) = vals.get_mut("flow_tracking_data") {
                vals.remove("flow_tracking_data");
            }

            Ok(())
        })?;

        // Handle a backwards compatibility hack which is required if compiler_version
        // is not present. See `insert_space` for more details.
        if self.compiler_version.is_none() {
            backward_compatibility::add_extra_space_to_cairo_named_tuples(&mut self.identifiers);
            backward_compatibility::add_extra_space_to_cairo_named_tuples(&mut self.reference_manager);
        }

        Ok(())
    }
}

// Modify the json content to match the python implementation
mod backward_compatibility {
    pub fn add_extra_space_to_cairo_named_tuples(value: &mut serde_json::Value) {
        match value {
            serde_json::Value::Array(v) => walk_array(v),
            serde_json::Value::Object(m) => walk_map(m),
            _ => {}
        }
    }
    fn walk_array(array: &mut [serde_json::Value]) {
        for v in array.iter_mut() {
            add_extra_space_to_cairo_named_tuples(v);
        }
    }

    fn walk_map(object: &mut serde_json::Map<String, serde_json::Value>) {
        for (k, v) in object.iter_mut() {
            match v {
                serde_json::Value::String(s) => {
                    let new_value = add_extra_space_to_named_tuple_type_definition(k, s);
                    if new_value.as_ref() != s {
                        *v = serde_json::Value::String(new_value.into());
                    }
                }
                _ => add_extra_space_to_cairo_named_tuples(v),
            }
        }
    }

    fn add_extra_space_to_named_tuple_type_definition<'a>(key: &str, value: &'a str) -> std::borrow::Cow<'a, str> {
        use std::borrow::Cow::*;
        match key {
            "cairo_type" | "value" => Owned(add_extra_space_before_colon(value)),
            _ => Borrowed(value),
        }
    }

    fn add_extra_space_before_colon(v: &str) -> String {
        // This is required because if we receive an already correct ` : `, we will
        // still "repair" it to `  : ` which we then fix at the end.
        v.replace(": ", " : ").replace("  :", " :")
    }
}
