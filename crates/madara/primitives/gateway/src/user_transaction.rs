//! User transactions type for the gateway.
//!
//! This module handles user transactions that are sent to or received by the gateway.
//! It defines the structure and conversion logic for different types of user transactions.
//!
//! //! # Important Note
//!
//! Query-only transactions are intentionally not supported in this module. This is because
//! UserTransactions are specifically designed for transactions that are meant to be added
//! to the sequencer's mempool via the gateway. Query transactions, which are executed
//! without affecting the chain state, are handled through different pathways.
//!
//! # Transaction Types
//!
//! The module supports three main types of transactions:
//! - [`UserDeclareTransaction`] - For declaring new contracts
//! - [`UserInvokeFunctionTransaction`] - For invoking functions on existing contracts
//! - [`UserDeployAccountTransaction`] - For deploying new account contracts
//!
//! # Features
//!
//! - Conversion between user and broadcasted transaction formats
//! - Serialization/deserialization using serde
//! - Error handling for invalid transaction types
//!
//! # Error Handling
//!
//! The module defines [`UserTransactionConversionError`] for handling conversion failures:
//!
//! - `UnsupportedQueryTransaction`: When attempting to convert a query-only transaction
//! - `ContractClassDecodeError`: When contract class decoding fails
//!

use mp_class::{CompressedLegacyContractClass, CompressedSierraClass, FlattenedSierraClass};
use mp_convert::hex_serde::U64AsHex;
use mp_transactions::{DataAvailabilityMode, ResourceBoundsMapping};
use serde::{Deserialize, Serialize};
use serde_with::serde_as;
use starknet_types_core::felt::Felt;
use starknet_types_rpc::{
    BroadcastedDeclareTxn, BroadcastedDeclareTxnV1, BroadcastedDeclareTxnV2, BroadcastedDeclareTxnV3,
    BroadcastedDeployAccountTxn, BroadcastedInvokeTxn, BroadcastedTxn, DeployAccountTxnV1, DeployAccountTxnV3,
    InvokeTxnV0, InvokeTxnV1, InvokeTxnV3,
};

#[derive(Debug, thiserror::Error)]
pub enum UserTransactionConversionError {
    #[error("User transaction can't be a query only transaction")]
    UnsupportedQueryTransaction,
    #[error("Error while decoding the contract class: {0}")]
    ContractClassDecodeError(#[from] std::io::Error),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
#[serde(deny_unknown_fields)]
pub enum UserTransaction {
    Declare(UserDeclareTransaction),
    InvokeFunction(UserInvokeFunctionTransaction),
    DeployAccount(UserDeployAccountTransaction),
}

impl TryFrom<UserTransaction> for BroadcastedTxn<Felt> {
    type Error = UserTransactionConversionError;

    fn try_from(transaction: UserTransaction) -> Result<Self, Self::Error> {
        match transaction {
            UserTransaction::Declare(tx) => Ok(BroadcastedTxn::Declare(tx.try_into()?)),
            UserTransaction::InvokeFunction(tx) => Ok(BroadcastedTxn::Invoke(tx.into())),
            UserTransaction::DeployAccount(tx) => Ok(BroadcastedTxn::DeployAccount(tx.into())),
        }
    }
}

impl TryFrom<BroadcastedTxn<Felt>> for UserTransaction {
    type Error = UserTransactionConversionError;

    fn try_from(transaction: BroadcastedTxn<Felt>) -> Result<Self, Self::Error> {
        match transaction {
            BroadcastedTxn::Declare(tx) => Ok(UserTransaction::Declare(tx.try_into()?)),
            BroadcastedTxn::Invoke(tx) => Ok(UserTransaction::InvokeFunction(tx.try_into()?)),
            BroadcastedTxn::DeployAccount(tx) => Ok(UserTransaction::DeployAccount(tx.try_into()?)),
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

impl TryFrom<UserDeclareTransaction> for BroadcastedDeclareTxn<Felt> {
    type Error = UserTransactionConversionError;

    fn try_from(transaction: UserDeclareTransaction) -> Result<Self, Self::Error> {
        match transaction {
            UserDeclareTransaction::V1(tx) => Ok(BroadcastedDeclareTxn::V1(tx.into())),
            UserDeclareTransaction::V2(tx) => Ok(BroadcastedDeclareTxn::V2(tx.try_into()?)),
            UserDeclareTransaction::V3(tx) => Ok(BroadcastedDeclareTxn::V3(tx.try_into()?)),
        }
    }
}

impl TryFrom<BroadcastedDeclareTxn<Felt>> for UserDeclareTransaction {
    type Error = UserTransactionConversionError;

    fn try_from(transaction: BroadcastedDeclareTxn<Felt>) -> Result<Self, Self::Error> {
        match transaction {
            BroadcastedDeclareTxn::V1(tx) => Ok(UserDeclareTransaction::V1(tx.try_into()?)),
            BroadcastedDeclareTxn::V2(tx) => Ok(UserDeclareTransaction::V2(tx.try_into()?)),
            BroadcastedDeclareTxn::V3(tx) => Ok(UserDeclareTransaction::V3(tx.try_into()?)),
            BroadcastedDeclareTxn::QueryV1(_)
            | BroadcastedDeclareTxn::QueryV2(_)
            | BroadcastedDeclareTxn::QueryV3(_) => Err(UserTransactionConversionError::UnsupportedQueryTransaction),
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
}

impl From<UserDeclareV1Transaction> for BroadcastedDeclareTxnV1<Felt> {
    fn from(transaction: UserDeclareV1Transaction) -> Self {
        Self {
            sender_address: transaction.sender_address,
            max_fee: transaction.max_fee,
            signature: transaction.signature,
            nonce: transaction.nonce,
            contract_class: transaction.contract_class.into(),
        }
    }
}

impl TryFrom<BroadcastedDeclareTxnV1<Felt>> for UserDeclareV1Transaction {
    type Error = std::io::Error;

    fn try_from(transaction: BroadcastedDeclareTxnV1<Felt>) -> Result<Self, Self::Error> {
        Ok(Self {
            sender_address: transaction.sender_address,
            max_fee: transaction.max_fee,
            signature: transaction.signature,
            nonce: transaction.nonce,
            contract_class: transaction.contract_class.try_into()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserDeclareV2Transaction {
    pub contract_class: CompressedSierraClass,
    pub compiled_class_hash: Felt,
    pub sender_address: Felt,
    pub max_fee: Felt,
    pub signature: Vec<Felt>,
    pub nonce: Felt,
}

impl TryFrom<UserDeclareV2Transaction> for BroadcastedDeclareTxnV2<Felt> {
    type Error = std::io::Error;

    fn try_from(transaction: UserDeclareV2Transaction) -> Result<Self, Self::Error> {
        let flattened_sierra_class: FlattenedSierraClass = transaction.contract_class.try_into()?;

        Ok(Self {
            sender_address: transaction.sender_address,
            compiled_class_hash: transaction.compiled_class_hash,
            max_fee: transaction.max_fee,
            signature: transaction.signature,
            nonce: transaction.nonce,
            contract_class: flattened_sierra_class.into(),
        })
    }
}

impl TryFrom<BroadcastedDeclareTxnV2<Felt>> for UserDeclareV2Transaction {
    type Error = std::io::Error;

    fn try_from(transaction: BroadcastedDeclareTxnV2<Felt>) -> Result<Self, Self::Error> {
        let flattened_sierra_class: FlattenedSierraClass = transaction.contract_class.into();

        Ok(Self {
            sender_address: transaction.sender_address,
            compiled_class_hash: transaction.compiled_class_hash,
            max_fee: transaction.max_fee,
            signature: transaction.signature,
            nonce: transaction.nonce,
            contract_class: flattened_sierra_class.try_into()?,
        })
    }
}

#[serde_as]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserDeclareV3Transaction {
    pub contract_class: CompressedSierraClass,
    pub compiled_class_hash: Felt,
    pub sender_address: Felt,
    pub signature: Vec<Felt>,
    pub nonce: Felt,
    pub nonce_data_availability_mode: DataAvailabilityMode,
    pub fee_data_availability_mode: DataAvailabilityMode,
    pub resource_bounds: ResourceBoundsMapping,
    #[serde_as(as = "U64AsHex")]
    pub tip: u64,
    pub paymaster_data: Vec<Felt>,
    pub account_deployment_data: Vec<Felt>,
}

impl TryFrom<UserDeclareV3Transaction> for BroadcastedDeclareTxnV3<Felt> {
    type Error = std::io::Error;

    fn try_from(transaction: UserDeclareV3Transaction) -> Result<Self, Self::Error> {
        let flattened_sierra_class: FlattenedSierraClass = transaction.contract_class.try_into()?;

        Ok(Self {
            sender_address: transaction.sender_address,
            compiled_class_hash: transaction.compiled_class_hash,
            signature: transaction.signature,
            nonce: transaction.nonce,
            nonce_data_availability_mode: transaction.nonce_data_availability_mode.into(),
            fee_data_availability_mode: transaction.fee_data_availability_mode.into(),
            resource_bounds: transaction.resource_bounds.into(),
            tip: transaction.tip,
            contract_class: flattened_sierra_class.into(),
            paymaster_data: transaction.paymaster_data,
            account_deployment_data: transaction.account_deployment_data,
        })
    }
}

impl TryFrom<BroadcastedDeclareTxnV3<Felt>> for UserDeclareV3Transaction {
    type Error = std::io::Error;

    fn try_from(transaction: BroadcastedDeclareTxnV3<Felt>) -> Result<Self, Self::Error> {
        let flattened_sierra_class: FlattenedSierraClass = transaction.contract_class.into();

        Ok(Self {
            sender_address: transaction.sender_address,
            compiled_class_hash: transaction.compiled_class_hash,
            signature: transaction.signature,
            nonce: transaction.nonce,
            nonce_data_availability_mode: transaction.nonce_data_availability_mode.into(),
            fee_data_availability_mode: transaction.fee_data_availability_mode.into(),
            resource_bounds: transaction.resource_bounds.into(),
            tip: transaction.tip,
            contract_class: flattened_sierra_class.try_into()?,
            paymaster_data: transaction.paymaster_data,
            account_deployment_data: transaction.account_deployment_data,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "version")]
pub enum UserInvokeFunctionTransaction {
    #[serde(rename = "0x0")]
    V0(UserInvokeFunctionV0Transaction),
    #[serde(rename = "0x1")]
    V1(UserInvokeFunctionV1Transaction),
    #[serde(rename = "0x3")]
    V3(UserInvokeFunctionV3Transaction),
}

impl From<UserInvokeFunctionTransaction> for BroadcastedInvokeTxn<Felt> {
    fn from(transaction: UserInvokeFunctionTransaction) -> Self {
        match transaction {
            UserInvokeFunctionTransaction::V0(tx) => BroadcastedInvokeTxn::V0(tx.into()),
            UserInvokeFunctionTransaction::V1(tx) => BroadcastedInvokeTxn::V1(tx.into()),
            UserInvokeFunctionTransaction::V3(tx) => BroadcastedInvokeTxn::V3(tx.into()),
        }
    }
}

impl TryFrom<BroadcastedInvokeTxn<Felt>> for UserInvokeFunctionTransaction {
    type Error = UserTransactionConversionError;

    fn try_from(transaction: BroadcastedInvokeTxn<Felt>) -> Result<Self, Self::Error> {
        match transaction {
            BroadcastedInvokeTxn::V0(tx) => Ok(UserInvokeFunctionTransaction::V0(tx.into())),
            BroadcastedInvokeTxn::V1(tx) => Ok(UserInvokeFunctionTransaction::V1(tx.into())),
            BroadcastedInvokeTxn::V3(tx) => Ok(UserInvokeFunctionTransaction::V3(tx.into())),
            BroadcastedInvokeTxn::QueryV0(_) | BroadcastedInvokeTxn::QueryV1(_) | BroadcastedInvokeTxn::QueryV3(_) => {
                Err(UserTransactionConversionError::UnsupportedQueryTransaction)
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserInvokeFunctionV0Transaction {
    pub sender_address: Felt,
    pub entry_point_selector: Felt,
    pub calldata: Vec<Felt>,
    pub signature: Vec<Felt>,
    pub max_fee: Felt,
}

impl From<UserInvokeFunctionV0Transaction> for InvokeTxnV0<Felt> {
    fn from(transaction: UserInvokeFunctionV0Transaction) -> Self {
        Self {
            contract_address: transaction.sender_address,
            entry_point_selector: transaction.entry_point_selector,
            calldata: transaction.calldata,
            signature: transaction.signature,
            max_fee: transaction.max_fee,
        }
    }
}

impl From<InvokeTxnV0<Felt>> for UserInvokeFunctionV0Transaction {
    fn from(transaction: InvokeTxnV0<Felt>) -> Self {
        Self {
            sender_address: transaction.contract_address,
            entry_point_selector: transaction.entry_point_selector,
            calldata: transaction.calldata,
            signature: transaction.signature,
            max_fee: transaction.max_fee,
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
}

impl From<UserInvokeFunctionV1Transaction> for InvokeTxnV1<Felt> {
    fn from(transaction: UserInvokeFunctionV1Transaction) -> Self {
        Self {
            sender_address: transaction.sender_address,
            calldata: transaction.calldata,
            signature: transaction.signature,
            max_fee: transaction.max_fee,
            nonce: transaction.nonce,
        }
    }
}

impl From<InvokeTxnV1<Felt>> for UserInvokeFunctionV1Transaction {
    fn from(transaction: InvokeTxnV1<Felt>) -> Self {
        Self {
            sender_address: transaction.sender_address,
            calldata: transaction.calldata,
            signature: transaction.signature,
            max_fee: transaction.max_fee,
            nonce: transaction.nonce,
        }
    }
}

#[serde_as]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserInvokeFunctionV3Transaction {
    pub sender_address: Felt,
    pub calldata: Vec<Felt>,
    pub signature: Vec<Felt>,
    pub nonce: Felt,
    pub nonce_data_availability_mode: DataAvailabilityMode,
    pub fee_data_availability_mode: DataAvailabilityMode,
    pub resource_bounds: ResourceBoundsMapping,
    #[serde_as(as = "U64AsHex")]
    pub tip: u64,
    pub paymaster_data: Vec<Felt>,
    pub account_deployment_data: Vec<Felt>,
}

impl From<UserInvokeFunctionV3Transaction> for InvokeTxnV3<Felt> {
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
        }
    }
}

impl From<InvokeTxnV3<Felt>> for UserInvokeFunctionV3Transaction {
    fn from(transaction: InvokeTxnV3<Felt>) -> Self {
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

impl From<UserDeployAccountTransaction> for BroadcastedDeployAccountTxn<Felt> {
    fn from(transaction: UserDeployAccountTransaction) -> Self {
        match transaction {
            UserDeployAccountTransaction::V1(tx) => BroadcastedDeployAccountTxn::V1(tx.into()),
            UserDeployAccountTransaction::V3(tx) => BroadcastedDeployAccountTxn::V3(tx.into()),
        }
    }
}

impl TryFrom<BroadcastedDeployAccountTxn<Felt>> for UserDeployAccountTransaction {
    type Error = UserTransactionConversionError;

    fn try_from(transaction: BroadcastedDeployAccountTxn<Felt>) -> Result<Self, Self::Error> {
        match transaction {
            BroadcastedDeployAccountTxn::V1(tx) => Ok(UserDeployAccountTransaction::V1(tx.into())),
            BroadcastedDeployAccountTxn::V3(tx) => Ok(UserDeployAccountTransaction::V3(tx.into())),
            BroadcastedDeployAccountTxn::QueryV1(_) | BroadcastedDeployAccountTxn::QueryV3(_) => {
                Err(UserTransactionConversionError::UnsupportedQueryTransaction)
            }
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
}

impl From<UserDeployAccountV1Transaction> for DeployAccountTxnV1<Felt> {
    fn from(transaction: UserDeployAccountV1Transaction) -> Self {
        Self {
            class_hash: transaction.class_hash,
            contract_address_salt: transaction.contract_address_salt,
            constructor_calldata: transaction.constructor_calldata,
            max_fee: transaction.max_fee,
            signature: transaction.signature,
            nonce: transaction.nonce,
        }
    }
}

impl From<DeployAccountTxnV1<Felt>> for UserDeployAccountV1Transaction {
    fn from(transaction: DeployAccountTxnV1<Felt>) -> Self {
        Self {
            class_hash: transaction.class_hash,
            contract_address_salt: transaction.contract_address_salt,
            constructor_calldata: transaction.constructor_calldata,
            max_fee: transaction.max_fee,
            signature: transaction.signature,
            nonce: transaction.nonce,
        }
    }
}

#[serde_as]
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
    #[serde_as(as = "U64AsHex")]
    pub tip: u64,
    pub paymaster_data: Vec<Felt>,
}

impl From<UserDeployAccountV3Transaction> for DeployAccountTxnV3<Felt> {
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
        }
    }
}

impl From<DeployAccountTxnV3<Felt>> for UserDeployAccountV3Transaction {
    fn from(transaction: DeployAccountTxnV3<Felt>) -> Self {
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
        }
    }
}
