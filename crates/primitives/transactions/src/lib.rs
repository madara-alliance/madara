pub mod compute_hash;
pub mod compute_hash_blockifier;
pub mod from_broadcasted_transactions;
mod from_starknet_provider;
pub mod getters;
mod to_starknet_api;
mod to_starknet_core;
pub mod utils;

use blockifier::transaction::account_transaction::AccountTransaction;

use starknet_types_core::felt::Felt;

const SIMULATE_TX_VERSION_OFFSET: Felt =
    Felt::from_raw([576460752142434320, 18446744073709551584, 17407, 18446744073700081665]);

/// Legacy check for deprecated txs
/// See `https://docs.starknet.io/documentation/architecture_and_concepts/Blocks/transactions/` for more details.

pub const LEGACY_BLOCK_NUMBER: u64 = 1470;
pub const LEGACY_L1_HANDLER_BLOCK: u64 = 854;

//  b"SN_MAIN" == 0x534e5f4d41494e
pub const MAIN_CHAIN_ID: Felt = Felt::from_hex_unchecked("0x534e5f4d41494e");

/// Wrapper type for transaction execution error.
/// Different tx types.
/// See `https://docs.starknet.io/documentation/architecture_and_concepts/Blocks/transactions/` for more details.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TxType {
    /// Regular invoke transaction.
    Invoke,
    /// Declare transaction.
    Declare,
    /// Deploy account transaction.
    DeployAccount,
    /// Message sent from ethereum.
    L1Handler,
}

impl From<TxType> for blockifier::transaction::transaction_types::TransactionType {
    fn from(value: TxType) -> Self {
        match value {
            TxType::Invoke => Self::InvokeFunction,
            TxType::Declare => Self::Declare,
            TxType::DeployAccount => Self::DeployAccount,
            TxType::L1Handler => Self::L1Handler,
        }
    }
}

impl From<&blockifier::transaction::transaction_execution::Transaction> for TxType {
    fn from(value: &blockifier::transaction::transaction_execution::Transaction) -> Self {
        match value {
            blockifier::transaction::transaction_execution::Transaction::AccountTransaction(tx) => tx.into(),
            blockifier::transaction::transaction_execution::Transaction::L1HandlerTransaction(_) => TxType::L1Handler,
        }
    }
}

impl From<&AccountTransaction> for TxType {
    fn from(value: &AccountTransaction) -> Self {
        match value {
            AccountTransaction::Declare(_) => TxType::Declare,
            AccountTransaction::DeployAccount(_) => TxType::DeployAccount,
            AccountTransaction::Invoke(_) => TxType::Invoke,
        }
    }
}

/////////////////////////// New transaction types ///////////////////////////

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Transaction {
    Invoke(InvokeTransaction),
    L1Handler(L1HandlerTransaction),
    Declare(DeclareTransaction),
    Deploy(DeployTransaction),
    DeployAccount(DeployAccountTransaction),
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum InvokeTransaction {
    V0(InvokeTransactionV0),
    V1(InvokeTransactionV1),
    V3(InvokeTransactionV3),
}

impl InvokeTransaction {
    pub fn sender_address(&self) -> Felt {
        match self {
            InvokeTransaction::V0(tx) => tx.contract_address,
            InvokeTransaction::V1(tx) => tx.sender_address,
            InvokeTransaction::V3(tx) => tx.sender_address,
        }
    }

    pub fn signature(&self) -> Vec<Felt> {
        match self {
            InvokeTransaction::V0(tx) => tx.signature.clone(),
            InvokeTransaction::V1(tx) => tx.signature.clone(),
            InvokeTransaction::V3(tx) => tx.signature.clone(),
        }
    }

    pub fn calldata(&self) -> Option<Vec<Felt>> {
        match self {
            InvokeTransaction::V0(tx) => Some(tx.calldata.clone()),
            InvokeTransaction::V1(tx) => Some(tx.calldata.clone()),
            InvokeTransaction::V3(tx) => Some(tx.calldata.clone()),
        }
    }

    pub fn nonce(&self) -> Felt {
        match self {
            InvokeTransaction::V0(_) => Felt::ZERO,
            InvokeTransaction::V1(tx) => tx.nonce,
            InvokeTransaction::V3(tx) => tx.nonce,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct InvokeTransactionV0 {
    pub transaction_hash: Felt,
    pub max_fee: Felt,
    pub signature: Vec<Felt>,
    pub contract_address: Felt,
    pub entry_point_selector: Felt,
    pub calldata: Vec<Felt>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct InvokeTransactionV1 {
    pub transaction_hash: Felt,
    pub sender_address: Felt,
    pub calldata: Vec<Felt>,
    pub max_fee: Felt,
    pub signature: Vec<Felt>,
    pub nonce: Felt,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct InvokeTransactionV3 {
    pub transaction_hash: Felt,
    pub sender_address: Felt,
    pub calldata: Vec<Felt>,
    pub signature: Vec<Felt>,
    pub nonce: Felt,
    pub resource_bounds: ResourceBoundsMapping,
    pub tip: u64,
    pub paymaster_data: Vec<Felt>,
    pub account_deployment_data: Vec<Felt>,
    pub nonce_data_availability_mode: DataAvailabilityMode,
    pub fee_data_availability_mode: DataAvailabilityMode,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct L1HandlerTransaction {
    pub transaction_hash: Felt,
    pub version: Felt,
    pub nonce: u64,
    pub contract_address: Felt,
    pub entry_point_selector: Felt,
    pub calldata: Vec<Felt>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum DeclareTransaction {
    V0(DeclareTransactionV0),
    V1(DeclareTransactionV1),
    V2(DeclareTransactionV2),
    V3(DeclareTransactionV3),
}

impl DeclareTransaction {
    pub fn sender_address(&self) -> Felt {
        match self {
            DeclareTransaction::V0(tx) => tx.sender_address,
            DeclareTransaction::V1(tx) => tx.sender_address,
            DeclareTransaction::V2(tx) => tx.sender_address,
            DeclareTransaction::V3(tx) => tx.sender_address,
        }
    }
    pub fn signature(&self) -> Vec<Felt> {
        match self {
            DeclareTransaction::V0(tx) => tx.signature.clone(),
            DeclareTransaction::V1(tx) => tx.signature.clone(),
            DeclareTransaction::V2(tx) => tx.signature.clone(),
            DeclareTransaction::V3(tx) => tx.signature.clone(),
        }
    }

    pub fn call_data(&self) -> Option<Vec<Felt>> {
        None
    }

    pub fn nonce(&self) -> Felt {
        match self {
            DeclareTransaction::V0(_) => Felt::ZERO,
            DeclareTransaction::V1(tx) => tx.nonce,
            DeclareTransaction::V2(tx) => tx.nonce,
            DeclareTransaction::V3(tx) => tx.nonce,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DeclareTransactionV0 {
    pub transaction_hash: Felt,
    pub sender_address: Felt,
    pub max_fee: Felt,
    pub signature: Vec<Felt>,
    pub class_hash: Felt,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DeclareTransactionV1 {
    pub transaction_hash: Felt,
    pub sender_address: Felt,
    pub max_fee: Felt,
    pub signature: Vec<Felt>,
    pub nonce: Felt,
    pub class_hash: Felt,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DeclareTransactionV2 {
    pub transaction_hash: Felt,
    pub sender_address: Felt,
    pub compiled_class_hash: Felt,
    pub max_fee: Felt,
    pub signature: Vec<Felt>,
    pub nonce: Felt,
    pub class_hash: Felt,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DeclareTransactionV3 {
    pub transaction_hash: Felt,
    pub sender_address: Felt,
    pub compiled_class_hash: Felt,
    pub signature: Vec<Felt>,
    pub nonce: Felt,
    pub class_hash: Felt,
    pub resource_bounds: ResourceBoundsMapping,
    pub tip: u64,
    pub paymaster_data: Vec<Felt>,
    pub account_deployment_data: Vec<Felt>,
    pub nonce_data_availability_mode: DataAvailabilityMode,
    pub fee_data_availability_mode: DataAvailabilityMode,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DeployTransaction {
    pub transaction_hash: Felt,
    pub version: Felt,
    pub contract_address_salt: Felt,
    pub constructor_calldata: Vec<Felt>,
    pub class_hash: Felt,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum DeployAccountTransaction {
    V1(DeployAccountTransactionV1),
    V3(DeployAccountTransactionV3),
}

impl DeployAccountTransaction {
    pub fn sender_address(&self) -> Felt {
        match self {
            DeployAccountTransaction::V1(tx) => tx.contract_address_salt,
            DeployAccountTransaction::V3(tx) => tx.contract_address_salt,
        }
    }
    pub fn signature(&self) -> Vec<Felt> {
        match self {
            DeployAccountTransaction::V1(tx) => tx.signature.clone(),
            DeployAccountTransaction::V3(tx) => tx.signature.clone(),
        }
    }

    pub fn calldata(&self) -> Option<Vec<Felt>> {
        match self {
            DeployAccountTransaction::V1(tx) => Some(tx.constructor_calldata.clone()),
            DeployAccountTransaction::V3(tx) => Some(tx.constructor_calldata.clone()),
        }
    }

    pub fn nonce(&self) -> Felt {
        match self {
            DeployAccountTransaction::V1(tx) => tx.nonce,
            DeployAccountTransaction::V3(tx) => tx.nonce,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DeployAccountTransactionV1 {
    pub transaction_hash: Felt,
    pub max_fee: Felt,
    pub signature: Vec<Felt>,
    pub nonce: Felt,
    pub contract_address_salt: Felt,
    pub constructor_calldata: Vec<Felt>,
    pub class_hash: Felt,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DeployAccountTransactionV3 {
    pub transaction_hash: Felt,
    pub signature: Vec<Felt>,
    pub nonce: Felt,
    pub contract_address_salt: Felt,
    pub constructor_calldata: Vec<Felt>,
    pub class_hash: Felt,
    pub resource_bounds: ResourceBoundsMapping,
    pub tip: u64,
    pub paymaster_data: Vec<Felt>,
    pub nonce_data_availability_mode: DataAvailabilityMode,
    pub fee_data_availability_mode: DataAvailabilityMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum DataAvailabilityMode {
    L1,
    L2,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]

pub struct ResourceBoundsMapping {
    pub l1_gas: ResourceBounds,
    pub l2_gas: ResourceBounds,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ResourceBounds {
    pub max_amount: u64,
    pub max_price_per_unit: u128,
}
