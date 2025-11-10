use strum::EnumIter;
use strum_macros::Display;

use super::constants::{
    EIC_CASM, EIC_SIERRA, ERC20_CASM, ERC20_SIERRA, MADARA_FACTORY_CASM, MADARA_FACTORY_SIERRA, TOKEN_BRIDGE_CASM,
    TOKEN_BRIDGE_SIERRA, UNIVERSAL_DEPLOYER_CASM, UNIVERSAL_DEPLOYER_SIERRA,
};

// Types for Map keys
#[derive(EnumIter, Debug, Display, Copy, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MadaraClass {
    TokenBridge,
    Erc20,
    Eic,
    UniversalDeployer,
    MadaraFactory,
}

impl MadaraClass {
    pub fn get_casm_path(self) -> &'static str {
        match self {
            MadaraClass::TokenBridge => TOKEN_BRIDGE_CASM,
            MadaraClass::Erc20 => ERC20_CASM,
            MadaraClass::Eic => EIC_CASM,
            MadaraClass::UniversalDeployer => UNIVERSAL_DEPLOYER_CASM,
            MadaraClass::MadaraFactory => MADARA_FACTORY_CASM,
        }
    }

    pub fn get_sierra_path(self) -> &'static str {
        match self {
            MadaraClass::TokenBridge => TOKEN_BRIDGE_SIERRA,
            MadaraClass::Erc20 => ERC20_SIERRA,
            MadaraClass::Eic => EIC_SIERRA,
            MadaraClass::UniversalDeployer => UNIVERSAL_DEPLOYER_SIERRA,
            MadaraClass::MadaraFactory => MADARA_FACTORY_SIERRA,
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MadaraClassInfo {
    pub madara_class: MadaraClass,
    pub sierra_path: &'static str,
    pub casm_path: &'static str,
}
