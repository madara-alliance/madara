use mp_class::{CompressedLegacyContractClass, FlattenedSierraClass};
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
#[serde(deny_unknown_fields)]
pub enum UserTransaction {
    Declare(UserDeclareTransaction),
    InvokeFunction(UserInvokeFunctionTransaction),
    DeployAccount(UserDeployAccountTransaction),
}

impl From<UserTransaction> for BroadcastedTxn<Felt> {
    fn from(transaction: UserTransaction) -> Self {
        match transaction {
            UserTransaction::Declare(tx) => BroadcastedTxn::Declare(tx.into()),
            UserTransaction::InvokeFunction(tx) => BroadcastedTxn::Invoke(tx.into()),
            UserTransaction::DeployAccount(tx) => BroadcastedTxn::DeployAccount(tx.into()),
        }
    }
}

impl TryFrom<BroadcastedTxn<Felt>> for UserTransaction {
    type Error = base64::DecodeError;

    fn try_from(transaction: BroadcastedTxn<Felt>) -> Result<Self, Self::Error> {
        match transaction {
            BroadcastedTxn::Declare(tx) => Ok(UserTransaction::Declare(tx.try_into()?)),
            BroadcastedTxn::Invoke(tx) => Ok(UserTransaction::InvokeFunction(tx.into())),
            BroadcastedTxn::DeployAccount(tx) => Ok(UserTransaction::DeployAccount(tx.into())),
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

impl From<UserDeclareTransaction> for BroadcastedDeclareTxn<Felt> {
    fn from(transaction: UserDeclareTransaction) -> Self {
        match transaction {
            UserDeclareTransaction::V1(tx) if tx.is_query => BroadcastedDeclareTxn::QueryV1(tx.into()),
            UserDeclareTransaction::V1(tx) => BroadcastedDeclareTxn::V1(tx.into()),
            UserDeclareTransaction::V2(tx) if tx.is_query => BroadcastedDeclareTxn::QueryV2(tx.into()),
            UserDeclareTransaction::V2(tx) => BroadcastedDeclareTxn::V2(tx.into()),
            UserDeclareTransaction::V3(tx) if tx.is_query => BroadcastedDeclareTxn::QueryV3(tx.into()),
            UserDeclareTransaction::V3(tx) => BroadcastedDeclareTxn::V3(tx.into()),
        }
    }
}

impl TryFrom<BroadcastedDeclareTxn<Felt>> for UserDeclareTransaction {
    type Error = base64::DecodeError;

    fn try_from(transaction: BroadcastedDeclareTxn<Felt>) -> Result<Self, Self::Error> {
        match transaction {
            BroadcastedDeclareTxn::V1(tx) => {
                Ok(UserDeclareTransaction::V1(UserDeclareV1Transaction::try_from_broadcasted(tx, false)?))
            }
            BroadcastedDeclareTxn::QueryV1(tx) => {
                Ok(UserDeclareTransaction::V1(UserDeclareV1Transaction::try_from_broadcasted(tx, true)?))
            }
            BroadcastedDeclareTxn::V2(tx) => {
                Ok(UserDeclareTransaction::V2(UserDeclareV2Transaction::from_broadcasted(tx, false)))
            }
            BroadcastedDeclareTxn::QueryV2(tx) => {
                Ok(UserDeclareTransaction::V2(UserDeclareV2Transaction::from_broadcasted(tx, true)))
            }
            BroadcastedDeclareTxn::V3(tx) => {
                Ok(UserDeclareTransaction::V3(UserDeclareV3Transaction::from_broadcasted(tx, false)))
            }
            BroadcastedDeclareTxn::QueryV3(tx) => {
                Ok(UserDeclareTransaction::V3(UserDeclareV3Transaction::from_broadcasted(tx, true)))
            }
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

impl UserDeclareV1Transaction {
    fn try_from_broadcasted(
        transaction: BroadcastedDeclareTxnV1<Felt>,
        is_query: bool,
    ) -> Result<Self, base64::DecodeError> {
        Ok(Self {
            sender_address: transaction.sender_address,
            max_fee: transaction.max_fee,
            signature: transaction.signature,
            nonce: transaction.nonce,
            contract_class: transaction.contract_class.try_into()?,
            is_query,
        })
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

impl From<UserDeclareV2Transaction> for BroadcastedDeclareTxnV2<Felt> {
    fn from(transaction: UserDeclareV2Transaction) -> Self {
        Self {
            sender_address: transaction.sender_address,
            compiled_class_hash: transaction.compiled_class_hash,
            max_fee: transaction.max_fee,
            signature: transaction.signature,
            nonce: transaction.nonce,
            contract_class: transaction.contract_class.into(),
        }
    }
}

impl UserDeclareV2Transaction {
    fn from_broadcasted(transaction: BroadcastedDeclareTxnV2<Felt>, is_query: bool) -> Self {
        Self {
            sender_address: transaction.sender_address,
            compiled_class_hash: transaction.compiled_class_hash,
            signature: transaction.signature,
            nonce: transaction.nonce,
            contract_class: transaction.contract_class.into(),
            max_fee: transaction.max_fee,
            is_query,
        }
    }
}

#[serde_as]
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
    #[serde_as(as = "U64AsHex")]
    pub tip: u64,
    pub paymaster_data: Vec<Felt>,
    pub account_deployment_data: Vec<Felt>,
    pub is_query: bool,
}

impl From<UserDeclareV3Transaction> for BroadcastedDeclareTxnV3<Felt> {
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
            contract_class: transaction.contract_class.into(),
            paymaster_data: transaction.paymaster_data,
            account_deployment_data: transaction.account_deployment_data,
        }
    }
}

impl UserDeclareV3Transaction {
    fn from_broadcasted(transaction: BroadcastedDeclareTxnV3<Felt>, is_query: bool) -> Self {
        Self {
            sender_address: transaction.sender_address,
            compiled_class_hash: transaction.compiled_class_hash,
            signature: transaction.signature,
            nonce: transaction.nonce,
            nonce_data_availability_mode: transaction.nonce_data_availability_mode.into(),
            fee_data_availability_mode: transaction.fee_data_availability_mode.into(),
            resource_bounds: transaction.resource_bounds.into(),
            tip: transaction.tip,
            contract_class: transaction.contract_class.into(),
            paymaster_data: transaction.paymaster_data,
            account_deployment_data: transaction.account_deployment_data,
            is_query,
        }
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
            UserInvokeFunctionTransaction::V0(tx) if tx.is_query => BroadcastedInvokeTxn::QueryV0(tx.into()),
            UserInvokeFunctionTransaction::V0(tx) => BroadcastedInvokeTxn::V0(tx.into()),
            UserInvokeFunctionTransaction::V1(tx) if tx.is_query => BroadcastedInvokeTxn::QueryV1(tx.into()),
            UserInvokeFunctionTransaction::V1(tx) => BroadcastedInvokeTxn::V1(tx.into()),
            UserInvokeFunctionTransaction::V3(tx) if tx.is_query => BroadcastedInvokeTxn::QueryV3(tx.into()),
            UserInvokeFunctionTransaction::V3(tx) => BroadcastedInvokeTxn::V3(tx.into()),
        }
    }
}

impl From<BroadcastedInvokeTxn<Felt>> for UserInvokeFunctionTransaction {
    fn from(transaction: BroadcastedInvokeTxn<Felt>) -> Self {
        match transaction {
            BroadcastedInvokeTxn::V0(tx) => {
                UserInvokeFunctionTransaction::V0(UserInvokeFunctionV0Transaction::from_broadcasted(tx, false))
            }
            BroadcastedInvokeTxn::QueryV0(tx) => {
                UserInvokeFunctionTransaction::V0(UserInvokeFunctionV0Transaction::from_broadcasted(tx, true))
            }
            BroadcastedInvokeTxn::V1(tx) => {
                UserInvokeFunctionTransaction::V1(UserInvokeFunctionV1Transaction::from_broadcasted(tx, false))
            }
            BroadcastedInvokeTxn::QueryV1(tx) => {
                UserInvokeFunctionTransaction::V1(UserInvokeFunctionV1Transaction::from_broadcasted(tx, true))
            }
            BroadcastedInvokeTxn::V3(tx) => {
                UserInvokeFunctionTransaction::V3(UserInvokeFunctionV3Transaction::from_broadcasted(tx, false))
            }
            BroadcastedInvokeTxn::QueryV3(tx) => {
                UserInvokeFunctionTransaction::V3(UserInvokeFunctionV3Transaction::from_broadcasted(tx, true))
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
    pub is_query: bool,
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

impl UserInvokeFunctionV0Transaction {
    fn from_broadcasted(transaction: InvokeTxnV0<Felt>, is_query: bool) -> Self {
        Self {
            sender_address: transaction.contract_address,
            entry_point_selector: transaction.entry_point_selector,
            calldata: transaction.calldata,
            signature: transaction.signature,
            max_fee: transaction.max_fee,
            is_query,
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

impl UserInvokeFunctionV1Transaction {
    fn from_broadcasted(transaction: InvokeTxnV1<Felt>, is_query: bool) -> Self {
        Self {
            sender_address: transaction.sender_address,
            calldata: transaction.calldata,
            signature: transaction.signature,
            max_fee: transaction.max_fee,
            nonce: transaction.nonce,
            is_query,
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
    pub is_query: bool,
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

impl UserInvokeFunctionV3Transaction {
    fn from_broadcasted(transaction: InvokeTxnV3<Felt>, is_query: bool) -> Self {
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
            is_query,
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
            UserDeployAccountTransaction::V1(tx) if tx.is_query => BroadcastedDeployAccountTxn::QueryV1(tx.into()),
            UserDeployAccountTransaction::V1(tx) => BroadcastedDeployAccountTxn::V1(tx.into()),
            UserDeployAccountTransaction::V3(tx) => BroadcastedDeployAccountTxn::QueryV3(tx.into()),
        }
    }
}

impl From<BroadcastedDeployAccountTxn<Felt>> for UserDeployAccountTransaction {
    fn from(transaction: BroadcastedDeployAccountTxn<Felt>) -> Self {
        match transaction {
            BroadcastedDeployAccountTxn::V1(tx) => {
                UserDeployAccountTransaction::V1(UserDeployAccountV1Transaction::from_broadcasted(tx, false))
            }
            BroadcastedDeployAccountTxn::V3(tx) => {
                UserDeployAccountTransaction::V3(UserDeployAccountV3Transaction::from_broadcasted(tx, false))
            }
            BroadcastedDeployAccountTxn::QueryV1(tx) => {
                UserDeployAccountTransaction::V1(UserDeployAccountV1Transaction::from_broadcasted(tx, true))
            }
            BroadcastedDeployAccountTxn::QueryV3(tx) => {
                UserDeployAccountTransaction::V3(UserDeployAccountV3Transaction::from_broadcasted(tx, true))
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
    pub is_query: bool,
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

impl UserDeployAccountV1Transaction {
    fn from_broadcasted(transaction: DeployAccountTxnV1<Felt>, is_query: bool) -> Self {
        Self {
            class_hash: transaction.class_hash,
            contract_address_salt: transaction.contract_address_salt,
            constructor_calldata: transaction.constructor_calldata,
            max_fee: transaction.max_fee,
            signature: transaction.signature,
            nonce: transaction.nonce,
            is_query,
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
    pub is_query: bool,
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

impl UserDeployAccountV3Transaction {
    fn from_broadcasted(transaction: DeployAccountTxnV3<Felt>, is_query: bool) -> Self {
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
            is_query,
        }
    }
}
