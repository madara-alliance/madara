// Types for Map keys
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MadaraClass {
    TokenBridge,
    Erc20,
    Eic,
    UniversalDeployer,
    MadaraFactory,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MadaraClassInfo {
    pub madara_class: MadaraClass,
    pub sierra_path: &'static str,
    pub casm_path: &'static str,
}

pub static MADARA_CLASSES_DATA: [MadaraClassInfo; 5] = [
    MadaraClassInfo {
        madara_class: MadaraClass::TokenBridge,
        sierra_path: "../build-artifacts/starkgate_latest/cairo/token_bridge.sierra.json",
        casm_path: "../build-artifacts/starkgate_latest/cairo/token_bridge.casm.json",
    },
    MadaraClassInfo {
        madara_class: MadaraClass::Erc20,
        sierra_path: "../build-artifacts/starkgate_latest/cairo/ERC20_070.sierra.json",
        casm_path: "../build-artifacts/starkgate_latest/cairo/ERC20_070.casm.json",
    },
    MadaraClassInfo {
        madara_class: MadaraClass::Eic,
        sierra_path: "./contracts/madara/target/dev/madara_factory_contracts_EIC.contract_class.json",
        casm_path: "./contracts/madara/target/dev/madara_factory_contracts_EIC.compiled_contract_class.json",
    },
    MadaraClassInfo {
        madara_class: MadaraClass::UniversalDeployer,
        sierra_path: "./contracts/madara/target/dev/madara_factory_contracts_UniversalDeployer.contract_class.json",
        casm_path: "./contracts/madara/target/dev/madara_factory_contracts_UniversalDeployer.compiled_contract_class.json",
    },
    MadaraClassInfo {
        madara_class: MadaraClass::MadaraFactory,
        sierra_path: "./contracts/madara/target/dev/madara_factory_contracts_MadaraFactory.contract_class.json",
        casm_path: "./contracts/madara/target/dev/madara_factory_contracts_MadaraFactory.compiled_contract_class.json",
    },
];
