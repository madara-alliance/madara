

// Types for Map keys
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImplementationContract {
    CoreContract,
    Manager,
    Registry,
    MultiBridge,
    EthBridge,
    EthBridgeEIC,
    BaseLayerFactory,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ImplementationContractInfo {
    pub implementation_contract: ImplementationContract,
    pub artifact_path: &'static str,
}

pub static IMPLEMENTATION_CONTRACTS_DATA: [ImplementationContractInfo; 6] = [
    ImplementationContractInfo {
        implementation_contract: ImplementationContract::CoreContract,
        artifact_path: "../build-artifacts/cairo_lang/Starknet.json",
    },
    ImplementationContractInfo {
        implementation_contract: ImplementationContract::Manager,
        artifact_path: "../build-artifacts/starkgate_latest/solidity/manager.json",
    },
    ImplementationContractInfo {
        implementation_contract: ImplementationContract::Registry,
        artifact_path: "../build-artifacts/starkgate_latest/solidity/registry.json",
    },
    ImplementationContractInfo {
        implementation_contract: ImplementationContract::MultiBridge,
        artifact_path: "../build-artifacts/starkgate_latest/solidity/multiBridge.json",
    },
    ImplementationContractInfo {
        implementation_contract: ImplementationContract::EthBridge,
        artifact_path: "../build-artifacts/starkgate_latest/solidity/ethBridge.json",
    },
    ImplementationContractInfo {
        implementation_contract: ImplementationContract::EthBridgeEIC,
        artifact_path: "./contracts/ethereum/out/ConfigureSingleBridgeEIC.sol/ConfigureSingleBridgeEIC.json",
    },
];
