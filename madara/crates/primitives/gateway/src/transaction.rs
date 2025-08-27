use std::sync::Arc;

use mp_convert::hex_serde::U64AsHex;
use mp_transactions::{DataAvailabilityMode, ResourceBoundsMapping};
use serde::{Deserialize, Serialize};
use serde_with::serde_as;
use starknet_types_core::felt::Felt;

type Signature = Arc<Vec<Felt>>;
type Calldata = Arc<Vec<Felt>>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
#[cfg_attr(feature = "deny_unknown_fields", serde(deny_unknown_fields))]
#[cfg_attr(test, derive(Eq))]
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

impl From<Transaction> for mp_transactions::Transaction {
    fn from(tx: Transaction) -> Self {
        match tx {
            Transaction::Invoke(tx) => Self::Invoke(tx.into()),
            Transaction::L1Handler(tx) => Self::L1Handler(tx.into()),
            Transaction::Declare(tx) => Self::Declare(tx.into()),
            Transaction::Deploy(tx) => Self::Deploy(tx.into()),
            Transaction::DeployAccount(tx) => Self::DeployAccount(tx.into()),
        }
    }
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

    pub fn version(&self) -> u8 {
        match self {
            Self::Invoke(tx) => tx.version(),
            Self::L1Handler(tx) => tx.version(),
            Self::Declare(tx) => tx.version(),
            Self::Deploy(tx) => tx.version(),
            Self::DeployAccount(tx) => tx.version(),
        }
    }

    pub fn transaction_hash(&self) -> &Felt {
        match self {
            Self::Invoke(tx) => tx.transaction_hash(),
            Self::L1Handler(tx) => &tx.transaction_hash,
            Self::Declare(tx) => tx.transaction_hash(),
            Self::Deploy(tx) => &tx.transaction_hash,
            Self::DeployAccount(tx) => tx.transaction_hash(),
        }
    }
    pub fn contract_address(&self) -> &Felt {
        match self {
            Self::Invoke(tx) => tx.contract_address(),
            Self::L1Handler(tx) => &tx.contract_address,
            Self::Declare(tx) => tx.contract_address(),
            Self::Deploy(tx) => &tx.contract_address,
            Self::DeployAccount(tx) => tx.contract_address(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "version")]
#[cfg_attr(feature = "deny_unknown_fields", serde(deny_unknown_fields))]
#[cfg_attr(test, derive(Eq))]
pub enum InvokeTransaction {
    #[serde(rename = "0x0")]
    V0(InvokeTransactionV0),
    #[serde(rename = "0x1")]
    V1(InvokeTransactionV1),
    #[serde(rename = "0x3")]
    V3(InvokeTransactionV3),
}

impl InvokeTransaction {
    pub fn version(&self) -> u8 {
        match self {
            InvokeTransaction::V0(_) => 0,
            InvokeTransaction::V1(_) => 1,
            InvokeTransaction::V3(_) => 3,
        }
    }
    
    pub fn transaction_hash(&self) -> &Felt {
        match self {
            Self::V0(tx) => &tx.transaction_hash,
            Self::V1(tx) => &tx.transaction_hash,
            Self::V3(tx) => &tx.transaction_hash,
        }
    }
    pub fn contract_address(&self) -> &Felt {
        match self {
            Self::V0(tx) => &tx.sender_address,
            Self::V1(tx) => &tx.sender_address,
            Self::V3(tx) => &tx.sender_address,
        }
    }
}

impl From<InvokeTransaction> for mp_transactions::InvokeTransaction {
    fn from(tx: InvokeTransaction) -> Self {
        match tx {
            InvokeTransaction::V0(tx) => Self::V0(tx.into()),
            InvokeTransaction::V1(tx) => Self::V1(tx.into()),
            InvokeTransaction::V3(tx) => Self::V3(tx.into()),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(feature = "deny_unknown_fields", serde(deny_unknown_fields))]
pub struct InvokeTransactionV0 {
    #[serde(rename = "contract_address")]
    pub sender_address: Felt,
    pub entry_point_selector: Felt,
    pub calldata: Calldata,
    pub signature: Signature,
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

impl From<InvokeTransactionV0> for mp_transactions::InvokeTransactionV0 {
    fn from(tx: InvokeTransactionV0) -> Self {
        Self {
            max_fee: tx.max_fee,
            signature: tx.signature,
            contract_address: tx.sender_address,
            entry_point_selector: tx.entry_point_selector,
            calldata: tx.calldata,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(feature = "deny_unknown_fields", serde(deny_unknown_fields))]
pub struct InvokeTransactionV1 {
    pub sender_address: Felt,
    pub calldata: Calldata,
    pub signature: Signature,
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

impl From<InvokeTransactionV1> for mp_transactions::InvokeTransactionV1 {
    fn from(tx: InvokeTransactionV1) -> Self {
        Self {
            sender_address: tx.sender_address,
            calldata: tx.calldata,
            max_fee: tx.max_fee,
            signature: tx.signature,
            nonce: tx.nonce,
        }
    }
}

#[serde_as]
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(feature = "deny_unknown_fields", serde(deny_unknown_fields))]
pub struct InvokeTransactionV3 {
    pub nonce: Felt,
    pub nonce_data_availability_mode: DataAvailabilityMode,
    pub fee_data_availability_mode: DataAvailabilityMode,
    pub resource_bounds: ResourceBoundsMapping,
    #[serde_as(as = "U64AsHex")]
    pub tip: u64,
    pub paymaster_data: Vec<Felt>,
    pub sender_address: Felt,
    pub signature: Signature,
    pub transaction_hash: Felt,
    pub calldata: Calldata,
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

impl From<InvokeTransactionV3> for mp_transactions::InvokeTransactionV3 {
    fn from(tx: InvokeTransactionV3) -> Self {
        Self {
            sender_address: tx.sender_address,
            calldata: tx.calldata,
            signature: tx.signature,
            nonce: tx.nonce,
            resource_bounds: tx.resource_bounds,
            tip: tx.tip,
            paymaster_data: tx.paymaster_data,
            account_deployment_data: tx.account_deployment_data,
            nonce_data_availability_mode: tx.nonce_data_availability_mode,
            fee_data_availability_mode: tx.fee_data_availability_mode,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(feature = "deny_unknown_fields", serde(deny_unknown_fields))]
pub struct L1HandlerTransaction {
    pub contract_address: Felt,
    pub entry_point_selector: Felt,
    #[serde(default)]
    pub nonce: Felt,
    pub calldata: Calldata,
    pub transaction_hash: Felt,
    pub version: Felt,
}

impl L1HandlerTransaction {
    pub fn version(&self) -> u8 {
        0
    }
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

impl From<L1HandlerTransaction> for mp_transactions::L1HandlerTransaction {
    fn from(tx: L1HandlerTransaction) -> Self {
        Self {
            version: tx.version,
            nonce: tx.nonce.try_into().unwrap_or_default(),
            contract_address: tx.contract_address,
            entry_point_selector: tx.entry_point_selector,
            calldata: tx.calldata,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "version")]
#[cfg_attr(test, derive(Eq))]
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

impl DeclareTransaction {
    pub fn version(&self) -> u8 {
        match self {
            DeclareTransaction::V0(_) => 0,
            DeclareTransaction::V1(_) => 1,
            DeclareTransaction::V2(_) => 2,
            DeclareTransaction::V3(_) => 3,
        }
    }
    
    pub fn transaction_hash(&self) -> &Felt {
        match self {
            Self::V0(tx) => &tx.transaction_hash,
            Self::V1(tx) => &tx.transaction_hash,
            Self::V2(tx) => &tx.transaction_hash,
            Self::V3(tx) => &tx.transaction_hash,
        }
    }
    pub fn contract_address(&self) -> &Felt {
        match self {
            Self::V0(tx) => &tx.sender_address,
            Self::V1(tx) => &tx.sender_address,
            Self::V2(tx) => &tx.sender_address,
            Self::V3(tx) => &tx.sender_address,
        }
    }
}

impl From<DeclareTransaction> for mp_transactions::DeclareTransaction {
    fn from(tx: DeclareTransaction) -> Self {
        match tx {
            DeclareTransaction::V0(tx) => Self::V0(tx.into()),
            DeclareTransaction::V1(tx) => Self::V1(tx.into()),
            DeclareTransaction::V2(tx) => Self::V2(tx.into()),
            DeclareTransaction::V3(tx) => Self::V3(tx.into()),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(feature = "deny_unknown_fields", serde(deny_unknown_fields))]
pub struct DeclareTransactionV0 {
    pub class_hash: Felt,
    pub max_fee: Felt,
    pub nonce: Felt,
    pub sender_address: Felt,
    #[serde(default)]
    pub signature: Signature,
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

impl From<DeclareTransactionV0> for mp_transactions::DeclareTransactionV0 {
    fn from(tx: DeclareTransactionV0) -> Self {
        Self {
            sender_address: tx.sender_address,
            max_fee: tx.max_fee,
            signature: tx.signature,
            class_hash: tx.class_hash,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(feature = "deny_unknown_fields", serde(deny_unknown_fields))]
pub struct DeclareTransactionV1 {
    pub class_hash: Felt,
    pub max_fee: Felt,
    pub nonce: Felt,
    pub sender_address: Felt,
    #[serde(default)]
    pub signature: Signature,
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

impl From<DeclareTransactionV1> for mp_transactions::DeclareTransactionV1 {
    fn from(tx: DeclareTransactionV1) -> Self {
        Self {
            sender_address: tx.sender_address,
            max_fee: tx.max_fee,
            signature: tx.signature,
            nonce: tx.nonce,
            class_hash: tx.class_hash,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(feature = "deny_unknown_fields", serde(deny_unknown_fields))]
pub struct DeclareTransactionV2 {
    pub class_hash: Felt,
    pub max_fee: Felt,
    pub nonce: Felt,
    pub sender_address: Felt,
    #[serde(default)]
    pub signature: Signature,
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

impl From<DeclareTransactionV2> for mp_transactions::DeclareTransactionV2 {
    fn from(tx: DeclareTransactionV2) -> Self {
        Self {
            sender_address: tx.sender_address,
            compiled_class_hash: tx.compiled_class_hash,
            max_fee: tx.max_fee,
            signature: tx.signature,
            nonce: tx.nonce,
            class_hash: tx.class_hash,
        }
    }
}

#[serde_as]
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(feature = "deny_unknown_fields", serde(deny_unknown_fields))]
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
    pub signature: Signature,
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

impl From<DeclareTransactionV3> for mp_transactions::DeclareTransactionV3 {
    fn from(tx: DeclareTransactionV3) -> Self {
        Self {
            sender_address: tx.sender_address,
            compiled_class_hash: tx.compiled_class_hash,
            signature: tx.signature,
            nonce: tx.nonce,
            class_hash: tx.class_hash,
            resource_bounds: tx.resource_bounds,
            tip: tx.tip,
            paymaster_data: tx.paymaster_data,
            account_deployment_data: tx.account_deployment_data,
            nonce_data_availability_mode: tx.nonce_data_availability_mode,
            fee_data_availability_mode: tx.fee_data_availability_mode,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(feature = "deny_unknown_fields", serde(deny_unknown_fields))]
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
    pub fn version(&self) -> u8 {
        0
    }
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

impl From<DeployTransaction> for mp_transactions::DeployTransaction {
    fn from(tx: DeployTransaction) -> Self {
        Self {
            version: tx.version,
            contract_address_salt: tx.contract_address_salt,
            constructor_calldata: tx.constructor_calldata,
            class_hash: tx.class_hash,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
#[cfg_attr(test, derive(Eq))]
pub enum DeployAccountTransaction {
    V1(DeployAccountTransactionV1),
    V3(DeployAccountTransactionV3),
}

impl DeployAccountTransaction {
    pub fn version(&self) -> u8 {
        match self {
            DeployAccountTransaction::V1(_) => 1,
            DeployAccountTransaction::V3(_) => 3,
        }
    }
    
    pub fn transaction_hash(&self) -> &Felt {
        match self {
            Self::V1(tx) => &tx.transaction_hash,
            Self::V3(tx) => &tx.transaction_hash,
        }
    }
    pub fn contract_address(&self) -> &Felt {
        match self {
            Self::V1(tx) => &tx.contract_address,
            Self::V3(tx) => &tx.sender_address,
        }
    }
}

impl From<DeployAccountTransaction> for mp_transactions::DeployAccountTransaction {
    fn from(tx: DeployAccountTransaction) -> Self {
        match tx {
            DeployAccountTransaction::V1(tx) => Self::V1(tx.into()),
            DeployAccountTransaction::V3(tx) => Self::V3(tx.into()),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(feature = "deny_unknown_fields", serde(deny_unknown_fields))]
pub struct DeployAccountTransactionV1 {
    pub contract_address: Felt,
    pub transaction_hash: Felt,
    pub max_fee: Felt,
    pub version: Felt,
    pub signature: Signature,
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

impl From<DeployAccountTransactionV1> for mp_transactions::DeployAccountTransactionV1 {
    fn from(tx: DeployAccountTransactionV1) -> Self {
        Self {
            max_fee: tx.max_fee,
            signature: tx.signature,
            nonce: tx.nonce,
            contract_address_salt: tx.contract_address_salt,
            constructor_calldata: tx.constructor_calldata,
            class_hash: tx.class_hash,
        }
    }
}

#[serde_as]
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(feature = "deny_unknown_fields", serde(deny_unknown_fields))]
pub struct DeployAccountTransactionV3 {
    pub nonce: Felt,
    pub nonce_data_availability_mode: DataAvailabilityMode,
    pub fee_data_availability_mode: DataAvailabilityMode,
    pub resource_bounds: ResourceBoundsMapping,
    #[serde_as(as = "U64AsHex")]
    pub tip: u64,
    pub paymaster_data: Vec<Felt>,
    pub sender_address: Felt,
    pub signature: Signature,
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

impl From<DeployAccountTransactionV3> for mp_transactions::DeployAccountTransactionV3 {
    fn from(tx: DeployAccountTransactionV3) -> Self {
        Self {
            signature: tx.signature,
            nonce: tx.nonce,
            contract_address_salt: tx.contract_address_salt,
            constructor_calldata: tx.constructor_calldata,
            class_hash: tx.class_hash,
            resource_bounds: tx.resource_bounds,
            tip: tx.tip,
            paymaster_data: tx.paymaster_data,
            nonce_data_availability_mode: tx.nonce_data_availability_mode,
            fee_data_availability_mode: tx.fee_data_availability_mode,
        }
    }
}
