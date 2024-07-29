// Credit: Pathfinder

use anyhow::Context;
use serde::Serialize;
use starknet_core::types::{
    Felt, FunctionStateMutability, LegacyEventAbiType, LegacyFunctionAbiType, LegacyStructAbiType, LegacyTypedParameter,
};

use super::cairo_program::CairoProgram;

/// Starkware doesn't use compact formatting for JSON but default python
/// formatting. This is required to hash to the same value after sorted
/// serialization.
pub struct PythonDefaultFormatter;

impl serde_json::ser::Formatter for PythonDefaultFormatter {
    fn begin_array_value<W>(&mut self, writer: &mut W, first: bool) -> std::io::Result<()>
    where
        W: ?Sized + std::io::Write,
    {
        if first {
            Ok(())
        } else {
            writer.write_all(b", ")
        }
    }

    fn begin_object_key<W>(&mut self, writer: &mut W, first: bool) -> std::io::Result<()>
    where
        W: ?Sized + std::io::Write,
    {
        if first {
            Ok(())
        } else {
            writer.write_all(b", ")
        }
    }

    fn begin_object_value<W>(&mut self, writer: &mut W) -> std::io::Result<()>
    where
        W: ?Sized + std::io::Write,
    {
        writer.write_all(b": ")
    }

    // Credit: Jonathan Lei from starknet-rs (https://github.com/xJonathanLEI/starknet-rs)`
    #[inline]
    fn write_string_fragment<W>(&mut self, writer: &mut W, fragment: &str) -> std::io::Result<()>
    where
        W: ?Sized + std::io::Write,
    {
        let mut buf = [0, 0];

        for c in fragment.chars() {
            if c.is_ascii() {
                writer.write_all(&[c as u8])?;
            } else {
                let buf = c.encode_utf16(&mut buf);
                for i in buf {
                    write!(writer, r"\u{:4x}", i)?;
                }
            }
        }

        Ok(())
    }

    // To save space serde_json deserialize some really big numbers as their scientific notation, eg: 1e26.
    // We have to expand them again to compute the hash.
    fn write_number_str<W>(&mut self, writer: &mut W, value: &str) -> std::io::Result<()>
    where
        W: ?Sized + std::io::Write,
    {
        let mut chars = value.chars();
        let mut e_pos = 0;
        // We assume we won't encounter any floats in the contract class,
        // so no need to check for a '.' character.
        chars.by_ref().take_while(|&c| c != 'e').try_for_each(|c| {
            e_pos += 1;
            writer.write_all(&[c as u8])
        })?;

        // We encountered the 'e' character, therefore that's a scientific notation.
        if e_pos != value.len() {
            let exponent = chars
                .as_str()
                .parse::<usize>()
                .map_err(|_| std::io::Error::other("could not parse exponent as usize"))?;
            for _ in 0..exponent {
                writer.write_all(&[b'0'])?
            }
        }

        Ok(())
    }
}

// This struct is serialized according to the python impl and the resulting bytes are fed to the starknet keccak hasher.
#[derive(Serialize)]
struct KeccakInput<'a, 'b> {
    #[serde(skip_serializing_if = "Option::is_none")]
    abi: Option<Vec<LegacyContractAbiEntry>>,
    #[serde(borrow)]
    program: &'a CairoProgram<'b>,
}

// Compute the cairo program's starknet-keccak hash in the same way it was done in the starknet python hints
pub fn compute_cairo_program_keccak(
    program: &CairoProgram,
    abi: &Option<Vec<starknet_core::types::LegacyContractAbiEntry>>,
) -> Result<Felt, anyhow::Error> {
    let keccak_input = KeccakInput { abi: abi.as_ref().map(|v| v.iter().map(|e| e.clone().into()).collect()), program };

    let mut string_buffer = vec![];

    let mut ser = serde_json::Serializer::with_formatter(&mut string_buffer, PythonDefaultFormatter);
    keccak_input.serialize(&mut ser).context("Serializing contract_definition for Keccak256")?;

    let raw_json_output = unsafe {
        // We never emit invalid UTF-8.
        String::from_utf8_unchecked(string_buffer)
    };

    Ok(starknet_core::utils::starknet_keccak(raw_json_output.as_bytes()))
}

// Everything that follows is only required because the python serialization originaly used guarantee that fields of each struct are serialized in alphabetical order.
// But:
// - Starknet-rs does not respect alphabetical order in it's struct, prompting the Serde derivation neither follow it.
// - Serde provide now way to specify that fields should be serialized this way.
//
// Note that I believe starknet-io does respect the alphabetical order and that we will be able to remove it when we swap lib.

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum LegacyContractAbiEntry {
    Function(LegacyFunctionAbiEntry),
    Event(LegacyEventAbiEntry),
    Struct(LegacyStructAbiEntry),
}

impl From<starknet_core::types::LegacyContractAbiEntry> for LegacyContractAbiEntry {
    fn from(value: starknet_core::types::LegacyContractAbiEntry) -> Self {
        match value {
            starknet_core::types::LegacyContractAbiEntry::Function(e) => Self::Function(e.into()),
            starknet_core::types::LegacyContractAbiEntry::Event(e) => Self::Event(e.into()),
            starknet_core::types::LegacyContractAbiEntry::Struct(e) => Self::Struct(e.into()),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct LegacyFunctionAbiEntry {
    /// Typed parameter
    pub inputs: Vec<LegacyTypedParameter>,
    /// The function name
    pub name: String,
    /// Typed parameter
    pub outputs: Vec<LegacyTypedParameter>,
    /// Function state mutability
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "stateMutability")]
    pub state_mutability: Option<FunctionStateMutability>,
    /// Function abi type
    pub r#type: LegacyFunctionAbiType,
}

impl From<starknet_core::types::LegacyFunctionAbiEntry> for LegacyFunctionAbiEntry {
    fn from(value: starknet_core::types::LegacyFunctionAbiEntry) -> Self {
        Self {
            inputs: value.inputs,
            name: value.name,
            outputs: value.outputs,
            state_mutability: value.state_mutability,
            r#type: value.r#type,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct LegacyStructAbiEntry {
    /// Members
    pub members: Vec<LegacyStructMember>,
    /// The struct name
    pub name: String,
    /// Size
    pub size: u64,
    /// Struct abi type
    pub r#type: LegacyStructAbiType,
}

impl From<starknet_core::types::LegacyStructAbiEntry> for LegacyStructAbiEntry {
    fn from(value: starknet_core::types::LegacyStructAbiEntry) -> Self {
        Self {
            members: value.members.into_iter().map(Into::into).collect(),
            name: value.name,
            size: value.size,
            r#type: value.r#type,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct LegacyStructMember {
    /// The parameter's name
    pub name: String,
    /// Offset of this property within the struct
    pub offset: u64,
    /// The parameter's type
    pub r#type: String,
}

impl From<starknet_core::types::LegacyStructMember> for LegacyStructMember {
    fn from(value: starknet_core::types::LegacyStructMember) -> Self {
        Self { name: value.name, offset: value.offset, r#type: value.r#type }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct LegacyEventAbiEntry {
    /// Typed parameter
    pub data: Vec<LegacyTypedParameter>,
    /// Typed parameter
    pub keys: Vec<LegacyTypedParameter>,
    /// The event name
    pub name: String,
    /// Event abi type
    pub r#type: LegacyEventAbiType,
}

impl From<starknet_core::types::LegacyEventAbiEntry> for LegacyEventAbiEntry {
    fn from(value: starknet_core::types::LegacyEventAbiEntry) -> Self {
        Self { data: value.data, keys: value.keys, name: value.name, r#type: value.r#type }
    }
}

#[cfg(test)]
mod test_serde_features {
    #[test]
    fn serde_json_value_sorts_maps() {
        // this property is leaned on and the default implementation of serde_json works
        // like this. serde_json has a feature called "preserve_order" which
        // could get enabled by accident, and it would destroy the ability
        // to compute_class_hash.

        let input = r#"{"foo": 1, "bar": 2}"#;
        let parsed = serde_json::from_str::<serde_json::Value>(input).unwrap();
        let output = serde_json::to_string(&parsed).unwrap();

        assert_eq!(output, r#"{"bar":2,"foo":1}"#);
    }

    #[test]
    fn serde_json_has_arbitrary_precision() {
        // the json has 251-bit ints, python handles them out of box, serde_json
        // requires feature "arbitrary_precision".

        // this is 2**256 - 1
        let input = r#"{"foo":115792089237316195423570985008687907853269984665640564039457584007913129639935}"#;

        let output = serde_json::to_string(&serde_json::from_str::<serde_json::Value>(input).unwrap()).unwrap();

        assert_eq!(input, output);
    }

    #[test]
    fn serde_json_has_raw_value() {
        // raw value is needed for others but here for completeness; this shouldn't
        // compile if you the feature wasn't enabled.

        #[derive(serde::Deserialize, serde::Serialize)]
        struct Program<'a> {
            #[serde(borrow)]
            debug_info: Option<&'a serde_json::value::RawValue>,
        }

        let mut input = serde_json::from_str::<Program<'_>>(
            r#"{"debug_info": {"long": {"tree": { "which": ["we dont", "care", "about", 0] }}}}"#,
        )
        .unwrap();

        input.debug_info = None;

        let output = serde_json::to_string(&input).unwrap();

        assert_eq!(output, r#"{"debug_info":null}"#);
    }
}
