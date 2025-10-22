use strum_macros::EnumIter;

use super::constants::{
    CORE_CONTRACT_ARTIFACT, ETH_BRIDGE_ARTIFACT, ETH_BRIDGE_EIC_ARTIFACT, MANAGER_ARTIFACT, MULTI_BRIDGE_ARTIFACT,
    REGISTRY_ARTIFACT,
};

// Types for Map keys
#[derive(EnumIter, Debug, Copy, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImplementationContract {
    CoreContract,
    Manager,
    Registry,
    MultiBridge,
    EthBridge,
    EthBridgeEIC,
}

pub trait GetArtifacts {
    fn get_artifact_path(&self) -> &'static str;
}

impl GetArtifacts for ImplementationContract {
    fn get_artifact_path(&self) -> &'static str {
        match self {
            ImplementationContract::CoreContract => CORE_CONTRACT_ARTIFACT,
            ImplementationContract::Manager => MANAGER_ARTIFACT,
            ImplementationContract::Registry => REGISTRY_ARTIFACT,
            ImplementationContract::MultiBridge => MULTI_BRIDGE_ARTIFACT,
            ImplementationContract::EthBridge => ETH_BRIDGE_ARTIFACT,
            ImplementationContract::EthBridgeEIC => ETH_BRIDGE_EIC_ARTIFACT,
        }
    }
}
