use std::sync::Arc;

use mp_class::{CompressedLegacyContractClass, FlattenedSierraClass};
use mp_transactions::{DataAvailabilityMode, ResourceBoundsMapping};
use serde::{Deserialize, Serialize};
use starknet_core::types::{
    BroadcastedDeclareTransaction, BroadcastedDeclareTransactionV1, BroadcastedDeclareTransactionV2,
    BroadcastedDeclareTransactionV3, BroadcastedDeployAccountTransaction, BroadcastedDeployAccountTransactionV1,
    BroadcastedDeployAccountTransactionV3, BroadcastedInvokeTransaction, BroadcastedInvokeTransactionV1,
    BroadcastedInvokeTransactionV3, BroadcastedTransaction,
};
use starknet_types_core::felt::Felt;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
#[serde(deny_unknown_fields)]
pub enum UserTransaction {
    DeclareV1(UserDeclareTransaction),
    InvokeFunction(UserInvokeFunctionTransaction),
    DeployAccount(UserDeployAccountTransaction),
}

impl From<UserTransaction> for BroadcastedTransaction {
    fn from(transaction: UserTransaction) -> Self {
        match transaction {
            UserTransaction::DeclareV1(v1) => BroadcastedTransaction::Declare(v1.into()),
            UserTransaction::InvokeFunction(v1) => BroadcastedTransaction::Invoke(v1.into()),
            UserTransaction::DeployAccount(v1) => BroadcastedTransaction::DeployAccount(v1.into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "version")]
pub enum UserDeclareTransaction {
    #[serde(rename = "0x1")]
    V1(UserDeclareV1Transaction),
    #[serde(rename = "0x2")]
    V2(UserDeclareV2Transaction),
    #[serde(rename = "0x3")]
    V3(UserDeclareV3Transaction),
}

impl From<UserDeclareTransaction> for BroadcastedDeclareTransaction {
    fn from(transaction: UserDeclareTransaction) -> Self {
        match transaction {
            UserDeclareTransaction::V1(v1) => BroadcastedDeclareTransaction::V1(v1.into()),
            UserDeclareTransaction::V2(v2) => BroadcastedDeclareTransaction::V2(v2.into()),
            UserDeclareTransaction::V3(v3) => BroadcastedDeclareTransaction::V3(v3.into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserDeclareV1Transaction {
    pub contract_class: CompressedLegacyContractClass,
    pub sender_address: Felt,
    pub max_fee: Felt,
    pub signature: Vec<Felt>,
    pub nonce: Felt,
    pub is_query: bool,
}

impl From<UserDeclareV1Transaction> for BroadcastedDeclareTransactionV1 {
    fn from(transaction: UserDeclareV1Transaction) -> Self {
        Self {
            sender_address: transaction.sender_address,
            max_fee: transaction.max_fee,
            signature: transaction.signature,
            nonce: transaction.nonce,
            contract_class: Arc::new(transaction.contract_class.into()),
            is_query: transaction.is_query,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserDeclareV2Transaction {
    pub contract_class: FlattenedSierraClass,
    pub compiled_class_hash: Felt,
    pub sender_address: Felt,
    pub max_fee: Felt,
    pub signature: Vec<Felt>,
    pub nonce: Felt,
    pub is_query: bool,
}

impl From<UserDeclareV2Transaction> for BroadcastedDeclareTransactionV2 {
    fn from(transaction: UserDeclareV2Transaction) -> Self {
        Self {
            sender_address: transaction.sender_address,
            compiled_class_hash: transaction.compiled_class_hash,
            max_fee: transaction.max_fee,
            signature: transaction.signature,
            nonce: transaction.nonce,
            contract_class: Arc::new(transaction.contract_class.into()),
            is_query: transaction.is_query,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserDeclareV3Transaction {
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

impl From<UserDeclareV3Transaction> for BroadcastedDeclareTransactionV3 {
    fn from(transaction: UserDeclareV3Transaction) -> Self {
        Self {
            sender_address: transaction.sender_address,
            compiled_class_hash: transaction.compiled_class_hash,
            signature: transaction.signature,
            nonce: transaction.nonce,
            nonce_data_availability_mode: transaction.nonce_data_availability_mode.into(),
            fee_data_availability_mode: transaction.fee_data_availability_mode.into(),
            resource_bounds: transaction.resource_bounds.into(),
            tip: transaction.tip,
            contract_class: Arc::new(transaction.contract_class.into()),
            paymaster_data: transaction.paymaster_data,
            account_deployment_data: transaction.account_deployment_data,
            is_query: transaction.is_query,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "version")]
pub enum UserInvokeFunctionTransaction {
    #[serde(rename = "0x1")]
    V1(UserInvokeFunctionV1Transaction),
    #[serde(rename = "0x3")]
    V3(UserInvokeFunctionV3Transaction),
}

impl From<UserInvokeFunctionTransaction> for BroadcastedInvokeTransaction {
    fn from(transaction: UserInvokeFunctionTransaction) -> Self {
        match transaction {
            UserInvokeFunctionTransaction::V1(v1) => BroadcastedInvokeTransaction::V1(v1.into()),
            UserInvokeFunctionTransaction::V3(v3) => BroadcastedInvokeTransaction::V3(v3.into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserInvokeFunctionV1Transaction {
    pub sender_address: Felt,
    pub calldata: Vec<Felt>,
    pub signature: Vec<Felt>,
    pub max_fee: Felt,
    pub nonce: Felt,
    pub is_query: bool,
}

impl From<UserInvokeFunctionV1Transaction> for BroadcastedInvokeTransactionV1 {
    fn from(transaction: UserInvokeFunctionV1Transaction) -> Self {
        Self {
            sender_address: transaction.sender_address,
            calldata: transaction.calldata,
            signature: transaction.signature,
            max_fee: transaction.max_fee,
            nonce: transaction.nonce,
            is_query: transaction.is_query,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserInvokeFunctionV3Transaction {
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

impl From<UserInvokeFunctionV3Transaction> for BroadcastedInvokeTransactionV3 {
    fn from(transaction: UserInvokeFunctionV3Transaction) -> Self {
        Self {
            sender_address: transaction.sender_address,
            calldata: transaction.calldata,
            signature: transaction.signature,
            nonce: transaction.nonce,
            nonce_data_availability_mode: transaction.nonce_data_availability_mode.into(),
            fee_data_availability_mode: transaction.fee_data_availability_mode.into(),
            resource_bounds: transaction.resource_bounds.into(),
            tip: transaction.tip,
            paymaster_data: transaction.paymaster_data,
            account_deployment_data: transaction.account_deployment_data,
            is_query: transaction.is_query,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "version")]
pub enum UserDeployAccountTransaction {
    #[serde(rename = "0x1")]
    V1(UserDeployAccountV1Transaction),
    #[serde(rename = "0x3")]
    V3(UserDeployAccountV3Transaction),
}

impl From<UserDeployAccountTransaction> for BroadcastedDeployAccountTransaction {
    fn from(transaction: UserDeployAccountTransaction) -> Self {
        match transaction {
            UserDeployAccountTransaction::V1(v1) => BroadcastedDeployAccountTransaction::V1(v1.into()),
            UserDeployAccountTransaction::V3(v3) => BroadcastedDeployAccountTransaction::V3(v3.into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserDeployAccountV1Transaction {
    pub class_hash: Felt,
    pub contract_address_salt: Felt,
    pub constructor_calldata: Vec<Felt>,
    pub max_fee: Felt,
    pub signature: Vec<Felt>,
    pub nonce: Felt,
    pub is_query: bool,
}

impl From<UserDeployAccountV1Transaction> for BroadcastedDeployAccountTransactionV1 {
    fn from(transaction: UserDeployAccountV1Transaction) -> Self {
        Self {
            class_hash: transaction.class_hash,
            contract_address_salt: transaction.contract_address_salt,
            constructor_calldata: transaction.constructor_calldata,
            max_fee: transaction.max_fee,
            signature: transaction.signature,
            nonce: transaction.nonce,
            is_query: transaction.is_query,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserDeployAccountV3Transaction {
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

impl From<UserDeployAccountV3Transaction> for BroadcastedDeployAccountTransactionV3 {
    fn from(transaction: UserDeployAccountV3Transaction) -> Self {
        Self {
            class_hash: transaction.class_hash,
            contract_address_salt: transaction.contract_address_salt,
            constructor_calldata: transaction.constructor_calldata,
            signature: transaction.signature,
            nonce: transaction.nonce,
            nonce_data_availability_mode: transaction.nonce_data_availability_mode.into(),
            fee_data_availability_mode: transaction.fee_data_availability_mode.into(),
            resource_bounds: transaction.resource_bounds.into(),
            tip: transaction.tip,
            paymaster_data: transaction.paymaster_data,
            is_query: transaction.is_query,
        }
    }
}
