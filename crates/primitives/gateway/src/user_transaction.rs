use mp_class::{CompressedLegacyContractClass, FlattenedSierraClass};
use mp_transactions::{DataAvailabilityMode, ResourceBoundsMapping};
use serde::{Deserialize, Serialize};
use starknet_types_core::felt::Felt;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
#[serde(deny_unknown_fields)]
pub enum UserTransaction {
    DeclareV1(DeclareTransaction),
    InvokeFunction(InvokeFunctionTransaction),
    DeployAccount(DeployAccountTransaction),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "version")]
pub enum DeclareTransaction {
    #[serde(rename = "0x1")]
    V1(DeclareV1Transaction),
    #[serde(rename = "0x2")]
    V2(DeclareV2Transaction),
    #[serde(rename = "0x3")]
    V3(DeclareV3Transaction),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeclareV1Transaction {
    pub contract_class: CompressedLegacyContractClass,
    pub sender_address: Felt,
    pub max_fee: Felt,
    pub signature: Vec<Felt>,
    pub nonce: Felt,
    pub is_query: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeclareV2Transaction {
    pub contract_class: FlattenedSierraClass,
    pub compiled_class_hash: Felt,
    pub sender_address: Felt,
    pub max_fee: Felt,
    pub signature: Vec<Felt>,
    pub nonce: Felt,
    pub is_query: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeclareV3Transaction {
    pub contract_class: FlattenedSierraClass,
    pub compiled_class_hash: Felt,
    pub sender_address: Felt,
    pub signature: Vec<Felt>,
    pub nonce: Felt,
    pub nonce_data_availability_mode: DataAvailabilityMode,
    pub fee_data_availability_mode: DataAvailabilityMode,
    pub resource_bounds: ResourceBoundsMapping,
    pub tip: u64,
    pub paymaster_data: Vec<Felt>,
    pub account_deployment_data: Vec<Felt>,
    pub is_query: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "version")]
pub enum InvokeFunctionTransaction {
    #[serde(rename = "0x1")]
    V1(InvokeFunctionV1Transaction),
    #[serde(rename = "0x3")]
    V3(InvokeFunctionV3Transaction),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InvokeFunctionV1Transaction {
    pub sender_address: Felt,
    pub calldata: Vec<Felt>,
    pub signature: Vec<Felt>,
    pub max_fee: Felt,
    pub nonce: Felt,
    pub is_query: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InvokeFunctionV3Transaction {
    pub sender_address: Felt,
    pub calldata: Vec<Felt>,
    pub signature: Vec<Felt>,
    pub nonce: Felt,
    pub nonce_data_availability_mode: DataAvailabilityMode,
    pub fee_data_availability_mode: DataAvailabilityMode,
    pub resource_bounds: ResourceBoundsMapping,
    pub tip: u64,
    pub paymaster_data: Vec<Felt>,
    pub account_deployment_data: Vec<Felt>,
    pub is_query: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "version")]
pub enum DeployAccountTransaction {
    #[serde(rename = "0x1")]
    V1(DeployAccountV1Transaction),
    #[serde(rename = "0x3")]
    V3(DeployAccountV3Transaction),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeployAccountV1Transaction {
    pub class_hash: Felt,
    pub contract_address_salt: Felt,
    pub constructor_calldata: Vec<Felt>,
    pub max_fee: Felt,
    pub signature: Vec<Felt>,
    pub nonce: Felt,
    pub is_query: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeployAccountV3Transaction {
    pub class_hash: Felt,
    pub contract_address_salt: Felt,
    pub constructor_calldata: Vec<Felt>,
    pub signature: Vec<Felt>,
    pub nonce: Felt,
    pub nonce_data_availability_mode: DataAvailabilityMode,
    pub fee_data_availability_mode: DataAvailabilityMode,
    pub resource_bounds: ResourceBoundsMapping,
    pub tip: u64,
    pub paymaster_data: Vec<Felt>,
    pub is_query: bool,
}
