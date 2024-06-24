mod compile;

#[derive(Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum CompiledClass {
    Sierra(CompiledSierra),
    Legacy(CompiledLegacy),
}

#[derive(Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CompiledSierra(Vec<u8>);

#[derive(Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CompiledLegacy(Vec<u8>);

pub fn to_blockifier_class(
    compiled_class: CompiledClass,
) -> anyhow::Result<blockifier::execution::contract_class::ContractClass> {
    match compiled_class {
        CompiledClass::Sierra(compiled_class) => Ok(blockifier::execution::contract_class::ContractClass::V1(
            blockifier::execution::contract_class::ContractClassV1::try_from_json_string(&String::from_utf8(
                compiled_class.0,
            )?)?,
        )),
        CompiledClass::Legacy(compiled_class) => Ok(blockifier::execution::contract_class::ContractClass::V0(
            blockifier::execution::contract_class::ContractClassV0::try_from_json_string(&String::from_utf8(
                compiled_class.0,
            )?)?,
        )),
    }
}
