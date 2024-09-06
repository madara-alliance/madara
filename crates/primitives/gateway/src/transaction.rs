use mp_convert::hex_serde::U64AsHex;
use mp_transactions::{DataAvailabilityMode, ResourceBoundsMapping};
use serde::{Deserialize, Serialize};
use serde_with::serde_as;
use starknet_types_core::felt::Felt;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
#[serde(deny_unknown_fields)]
pub enum Transaction {
    #[serde(rename = "INVOKE_FUNCTION")]
    Invoke(InvokeTransaction),
    #[serde(rename = "L1_HANDLER")]
    L1Handler(L1HandlerTransaction),
    #[serde(rename = "DECLARE")]
    Declare(DeclareTransaction),
    #[serde(rename = "DEPLOY")]
    Deploy(DeployTransaction),
    #[serde(rename = "DEPLOY_ACCOUNT")]
    DeployAccount(DeployAccountTransaction),
}

impl Transaction {
    pub fn new(
        mp_transactions::TransactionWithHash { transaction, hash }: mp_transactions::TransactionWithHash,
        contract_address: Option<Felt>,
    ) -> Self {
        match transaction {
            mp_transactions::Transaction::Invoke(mp_transactions::InvokeTransaction::V0(tx)) => {
                Transaction::Invoke(InvokeTransaction::V0(InvokeTransactionV0::new(tx, hash)))
            }
            mp_transactions::Transaction::Invoke(mp_transactions::InvokeTransaction::V1(tx)) => {
                Transaction::Invoke(InvokeTransaction::V1(InvokeTransactionV1::new(tx, hash)))
            }
            mp_transactions::Transaction::Invoke(mp_transactions::InvokeTransaction::V3(tx)) => {
                Transaction::Invoke(InvokeTransaction::V3(InvokeTransactionV3::new(tx, hash)))
            }
            mp_transactions::Transaction::L1Handler(tx) => Transaction::L1Handler(L1HandlerTransaction::new(tx, hash)),
            mp_transactions::Transaction::Declare(mp_transactions::DeclareTransaction::V0(tx)) => {
                Transaction::Declare(DeclareTransaction::V0(DeclareTransactionV0::new(tx, hash)))
            }
            mp_transactions::Transaction::Declare(mp_transactions::DeclareTransaction::V1(tx)) => {
                Transaction::Declare(DeclareTransaction::V1(DeclareTransactionV1::new(tx, hash)))
            }
            mp_transactions::Transaction::Declare(mp_transactions::DeclareTransaction::V2(tx)) => {
                Transaction::Declare(DeclareTransaction::V2(DeclareTransactionV2::new(tx, hash)))
            }
            mp_transactions::Transaction::Declare(mp_transactions::DeclareTransaction::V3(tx)) => {
                Transaction::Declare(DeclareTransaction::V3(DeclareTransactionV3::new(tx, hash)))
            }
            mp_transactions::Transaction::Deploy(tx) => {
                Transaction::Deploy(DeployTransaction::new(tx, hash, contract_address.unwrap_or_default()))
            }
            mp_transactions::Transaction::DeployAccount(mp_transactions::DeployAccountTransaction::V1(tx)) => {
                Transaction::DeployAccount(DeployAccountTransaction::V1(DeployAccountTransactionV1::new(
                    tx,
                    hash,
                    contract_address.unwrap_or_default(),
                )))
            }
            mp_transactions::Transaction::DeployAccount(mp_transactions::DeployAccountTransaction::V3(tx)) => {
                Transaction::DeployAccount(DeployAccountTransaction::V3(DeployAccountTransactionV3::new(
                    tx,
                    hash,
                    contract_address.unwrap_or_default(),
                )))
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "version")]
pub enum InvokeTransaction {
    #[serde(rename = "0x0")]
    V0(InvokeTransactionV0),
    #[serde(rename = "0x1")]
    V1(InvokeTransactionV1),
    #[serde(rename = "0x3")]
    V3(InvokeTransactionV3),
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct InvokeTransactionV0 {
    #[serde(alias = "contract_address")]
    pub sender_address: Felt,
    pub entry_point_selector: Felt,
    pub calldata: Vec<Felt>,
    pub signature: Vec<Felt>,
    pub max_fee: Felt,
    pub transaction_hash: Felt,
}

impl InvokeTransactionV0 {
    pub fn new(transaction: mp_transactions::InvokeTransactionV0, hash: Felt) -> Self {
        Self {
            sender_address: transaction.contract_address,
            entry_point_selector: transaction.entry_point_selector,
            calldata: transaction.calldata,
            signature: transaction.signature,
            max_fee: transaction.max_fee,
            transaction_hash: hash,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct InvokeTransactionV1 {
    pub sender_address: Felt,
    pub calldata: Vec<Felt>,
    pub signature: Vec<Felt>,
    pub max_fee: Felt,
    pub nonce: Felt,
    pub transaction_hash: Felt,
}

impl InvokeTransactionV1 {
    pub fn new(transaction: mp_transactions::InvokeTransactionV1, hash: Felt) -> Self {
        Self {
            sender_address: transaction.sender_address,
            calldata: transaction.calldata,
            signature: transaction.signature,
            max_fee: transaction.max_fee,
            nonce: transaction.nonce,
            transaction_hash: hash,
        }
    }
}

#[serde_as]
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct InvokeTransactionV3 {
    pub nonce: Felt,
    pub nonce_data_availability_mode: DataAvailabilityMode,
    pub fee_data_availability_mode: DataAvailabilityMode,
    pub resource_bounds: ResourceBoundsMapping,
    #[serde_as(as = "U64AsHex")]
    pub tip: u64,
    pub paymaster_data: Vec<Felt>,
    pub sender_address: Felt,
    pub signature: Vec<Felt>,
    pub transaction_hash: Felt,
    pub calldata: Vec<Felt>,
    pub account_deployment_data: Vec<Felt>,
}

impl InvokeTransactionV3 {
    pub fn new(transaction: mp_transactions::InvokeTransactionV3, hash: Felt) -> Self {
        Self {
            nonce: transaction.nonce,
            nonce_data_availability_mode: transaction.nonce_data_availability_mode,
            fee_data_availability_mode: transaction.fee_data_availability_mode,
            resource_bounds: transaction.resource_bounds,
            tip: transaction.tip,
            paymaster_data: transaction.paymaster_data,
            sender_address: transaction.sender_address,
            signature: transaction.signature,
            transaction_hash: hash,
            calldata: transaction.calldata,
            account_deployment_data: transaction.account_deployment_data,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct L1HandlerTransaction {
    pub contract_address: Felt,
    pub entry_point_selector: Felt,
    #[serde(default)]
    pub nonce: Felt,
    pub calldata: Vec<Felt>,
    pub transaction_hash: Felt,
    pub version: Felt,
}

impl L1HandlerTransaction {
    pub fn new(transaction: mp_transactions::L1HandlerTransaction, hash: Felt) -> Self {
        Self {
            contract_address: transaction.contract_address,
            entry_point_selector: transaction.entry_point_selector,
            nonce: transaction.nonce.into(),
            calldata: transaction.calldata,
            transaction_hash: hash,
            version: transaction.version,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "version")]
pub enum DeclareTransaction {
    #[serde(rename = "0x0")]
    V0(DeclareTransactionV0),
    #[serde(rename = "0x1")]
    V1(DeclareTransactionV1),
    #[serde(rename = "0x2")]
    V2(DeclareTransactionV2),
    #[serde(rename = "0x3")]
    V3(DeclareTransactionV3),
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DeclareTransactionV0 {
    pub class_hash: Felt,
    pub max_fee: Felt,
    pub nonce: Felt,
    pub sender_address: Felt,
    #[serde(default)]
    pub signature: Vec<Felt>,
    pub transaction_hash: Felt,
}

impl DeclareTransactionV0 {
    pub fn new(transaction: mp_transactions::DeclareTransactionV0, hash: Felt) -> Self {
        Self {
            class_hash: transaction.class_hash,
            max_fee: transaction.max_fee,
            nonce: Felt::ZERO,
            sender_address: transaction.sender_address,
            signature: transaction.signature,
            transaction_hash: hash,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DeclareTransactionV1 {
    pub class_hash: Felt,
    pub max_fee: Felt,
    pub nonce: Felt,
    pub sender_address: Felt,
    #[serde(default)]
    pub signature: Vec<Felt>,
    pub transaction_hash: Felt,
}

impl DeclareTransactionV1 {
    pub fn new(transaction: mp_transactions::DeclareTransactionV1, hash: Felt) -> Self {
        Self {
            class_hash: transaction.class_hash,
            max_fee: transaction.max_fee,
            nonce: transaction.nonce,
            sender_address: transaction.sender_address,
            signature: transaction.signature,
            transaction_hash: hash,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DeclareTransactionV2 {
    pub class_hash: Felt,
    pub max_fee: Felt,
    pub nonce: Felt,
    pub sender_address: Felt,
    #[serde(default)]
    pub signature: Vec<Felt>,
    pub transaction_hash: Felt,
    pub compiled_class_hash: Felt,
}

impl DeclareTransactionV2 {
    pub fn new(transaction: mp_transactions::DeclareTransactionV2, hash: Felt) -> Self {
        Self {
            class_hash: transaction.class_hash,
            max_fee: transaction.max_fee,
            nonce: transaction.nonce,
            sender_address: transaction.sender_address,
            signature: transaction.signature,
            transaction_hash: hash,
            compiled_class_hash: transaction.compiled_class_hash,
        }
    }
}

#[serde_as]
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DeclareTransactionV3 {
    pub class_hash: Felt,
    pub nonce: Felt,
    pub nonce_data_availability_mode: DataAvailabilityMode,
    pub fee_data_availability_mode: DataAvailabilityMode,
    pub resource_bounds: ResourceBoundsMapping,
    #[serde_as(as = "U64AsHex")]
    pub tip: u64,
    pub paymaster_data: Vec<Felt>,
    pub sender_address: Felt,
    #[serde(default)]
    pub signature: Vec<Felt>,
    pub transaction_hash: Felt,
    pub compiled_class_hash: Felt,
    pub account_deployment_data: Vec<Felt>,
}

impl DeclareTransactionV3 {
    pub fn new(transaction: mp_transactions::DeclareTransactionV3, hash: Felt) -> Self {
        Self {
            class_hash: transaction.class_hash,
            nonce: transaction.nonce,
            nonce_data_availability_mode: transaction.nonce_data_availability_mode,
            fee_data_availability_mode: transaction.fee_data_availability_mode,
            resource_bounds: transaction.resource_bounds,
            tip: transaction.tip,
            paymaster_data: transaction.paymaster_data,
            sender_address: transaction.sender_address,
            signature: transaction.signature,
            transaction_hash: hash,
            compiled_class_hash: transaction.compiled_class_hash,
            account_deployment_data: transaction.account_deployment_data,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DeployTransaction {
    pub constructor_calldata: Vec<Felt>,
    pub contract_address: Felt,
    pub contract_address_salt: Felt,
    pub class_hash: Felt,
    pub transaction_hash: Felt,
    #[serde(default)]
    pub version: Felt,
}

impl DeployTransaction {
    pub fn new(transaction: mp_transactions::DeployTransaction, hash: Felt, contract_address: Felt) -> Self {
        Self {
            constructor_calldata: transaction.constructor_calldata,
            contract_address,
            contract_address_salt: transaction.contract_address_salt,
            class_hash: transaction.class_hash,
            transaction_hash: hash,
            version: transaction.version,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DeployAccountTransaction {
    V1(DeployAccountTransactionV1),
    V3(DeployAccountTransactionV3),
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DeployAccountTransactionV1 {
    pub contract_address: Felt,
    pub transaction_hash: Felt,
    pub max_fee: Felt,
    pub version: Felt,
    pub signature: Vec<Felt>,
    pub nonce: Felt,
    pub contract_address_salt: Felt,
    pub constructor_calldata: Vec<Felt>,
    pub class_hash: Felt,
}

impl DeployAccountTransactionV1 {
    pub fn new(transaction: mp_transactions::DeployAccountTransactionV1, hash: Felt, contract_address: Felt) -> Self {
        Self {
            contract_address,
            transaction_hash: hash,
            max_fee: transaction.max_fee,
            version: Felt::ONE,
            signature: transaction.signature,
            nonce: transaction.nonce,
            contract_address_salt: transaction.contract_address_salt,
            constructor_calldata: transaction.constructor_calldata,
            class_hash: transaction.class_hash,
        }
    }
}

#[serde_as]
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DeployAccountTransactionV3 {
    pub nonce: Felt,
    pub nonce_data_availability_mode: DataAvailabilityMode,
    pub fee_data_availability_mode: DataAvailabilityMode,
    pub resource_bounds: ResourceBoundsMapping,
    #[serde_as(as = "U64AsHex")]
    pub tip: u64,
    pub paymaster_data: Vec<Felt>,
    pub sender_address: Felt,
    pub signature: Vec<Felt>,
    pub transaction_hash: Felt,
    pub version: Felt,
    pub contract_address_salt: Felt,
    pub constructor_calldata: Vec<Felt>,
    pub class_hash: Felt,
}

impl DeployAccountTransactionV3 {
    pub fn new(transaction: mp_transactions::DeployAccountTransactionV3, hash: Felt, sender_address: Felt) -> Self {
        Self {
            nonce: transaction.nonce,
            nonce_data_availability_mode: transaction.nonce_data_availability_mode,
            fee_data_availability_mode: transaction.fee_data_availability_mode,
            resource_bounds: transaction.resource_bounds,
            tip: transaction.tip,
            paymaster_data: transaction.paymaster_data,
            sender_address,
            signature: transaction.signature,
            transaction_hash: hash,
            version: Felt::THREE,
            contract_address_salt: transaction.contract_address_salt,
            constructor_calldata: transaction.constructor_calldata,
            class_hash: transaction.class_hash,
        }
    }
}
