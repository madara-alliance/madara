mod broadcasted_to_blockifier;
pub mod compute_hash;
mod from_blockifier;
mod from_broadcasted_transaction;
mod from_starknet_api;
mod from_starknet_provider;
mod to_starknet_api;
mod to_starknet_core;
pub mod utils;

use blockifier::transaction::objects::FeeType;
pub use broadcasted_to_blockifier::broadcasted_to_blockifier;
use dp_convert::ToFelt;
pub use from_starknet_provider::TransactionTypeError;
use starknet_api::transaction::TransactionVersion;
use starknet_types_core::{felt::Felt, hash::StarkHash};

const SIMULATE_TX_VERSION_OFFSET: Felt =
    Felt::from_raw([576460752142434320, 18446744073709551584, 17407, 18446744073700081665]);

/// Legacy check for deprecated txs
/// See `https://docs.starknet.io/documentation/architecture_and_concepts/Blocks/transactions/` for more details.

pub const LEGACY_BLOCK_NUMBER: u64 = 1470;
pub const V0_7_BLOCK_NUMBER: u64 = 833;

pub const MAIN_CHAIN_ID: Felt = Felt::from_hex_unchecked("0x0534e5f4d41494e"); // b"SN_MAIN"
pub const TEST_CHAIN_ID: Felt = Felt::from_hex_unchecked("0x0534e5f5345504f4c4941"); // b"SN_SEPOLIA"
pub const INTEGRATION_CHAIN_ID: Felt = Felt::from_hex_unchecked("0x0534e5f494e544547524154494f4e5f5345504f4c4941"); // b"SN_INTEGRATION_SEPOLIA"

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TransactionWithHash {
    pub transaction: Transaction,
    pub hash: Felt,
}

impl TransactionWithHash {
    pub fn new(transaction: Transaction, hash: Felt) -> Self {
        Self { transaction, hash }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Transaction {
    Invoke(InvokeTransaction),
    L1Handler(L1HandlerTransaction),
    Declare(DeclareTransaction),
    Deploy(DeployTransaction),
    DeployAccount(DeployAccountTransaction),
}

impl From<InvokeTransactionV0> for Transaction {
    fn from(tx: InvokeTransactionV0) -> Self {
        Transaction::Invoke(InvokeTransaction::V0(tx))
    }
}

impl From<InvokeTransactionV1> for Transaction {
    fn from(tx: InvokeTransactionV1) -> Self {
        Transaction::Invoke(InvokeTransaction::V1(tx))
    }
}

impl From<InvokeTransactionV3> for Transaction {
    fn from(tx: InvokeTransactionV3) -> Self {
        Transaction::Invoke(InvokeTransaction::V3(tx))
    }
}

impl From<L1HandlerTransaction> for Transaction {
    fn from(tx: L1HandlerTransaction) -> Self {
        Transaction::L1Handler(tx)
    }
}

impl From<DeclareTransactionV0> for Transaction {
    fn from(tx: DeclareTransactionV0) -> Self {
        Transaction::Declare(DeclareTransaction::V0(tx))
    }
}

impl From<DeclareTransactionV1> for Transaction {
    fn from(tx: DeclareTransactionV1) -> Self {
        Transaction::Declare(DeclareTransaction::V1(tx))
    }
}

impl From<DeclareTransactionV2> for Transaction {
    fn from(tx: DeclareTransactionV2) -> Self {
        Transaction::Declare(DeclareTransaction::V2(tx))
    }
}

impl From<DeclareTransactionV3> for Transaction {
    fn from(tx: DeclareTransactionV3) -> Self {
        Transaction::Declare(DeclareTransaction::V3(tx))
    }
}

impl From<DeployTransaction> for Transaction {
    fn from(tx: DeployTransaction) -> Self {
        Transaction::Deploy(tx)
    }
}

impl From<DeployAccountTransactionV1> for Transaction {
    fn from(tx: DeployAccountTransactionV1) -> Self {
        Transaction::DeployAccount(DeployAccountTransaction::V1(tx))
    }
}

impl From<DeployAccountTransactionV3> for Transaction {
    fn from(tx: DeployAccountTransactionV3) -> Self {
        Transaction::DeployAccount(DeployAccountTransaction::V3(tx))
    }
}

impl Transaction {
    pub fn version(&self) -> TransactionVersion {
        match self {
            Transaction::Invoke(tx) => tx.version(),
            Transaction::L1Handler(tx) => tx.version(),
            Transaction::Declare(tx) => tx.version(),
            Transaction::Deploy(tx) => tx.version(),
            Transaction::DeployAccount(tx) => tx.version(),
        }
    }

    pub fn is_l1_handler(&self) -> bool {
        matches!(self, Transaction::L1Handler(_))
    }

    /// Account transactions means everything except L1Handler.
    pub fn is_account(&self) -> bool {
        !matches!(self, Transaction::L1Handler(_))
    }

    pub fn fee_type(&self) -> FeeType {
        if self.is_l1_handler() || self.version() < TransactionVersion::THREE {
            FeeType::Eth
        } else {
            FeeType::Strk
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum InvokeTransaction {
    V0(InvokeTransactionV0),
    V1(InvokeTransactionV1),
    V3(InvokeTransactionV3),
}

impl InvokeTransaction {
    pub fn version(&self) -> TransactionVersion {
        match self {
            InvokeTransaction::V0(tx) => tx.version(),
            InvokeTransaction::V1(tx) => tx.version(),
            InvokeTransaction::V3(tx) => tx.version(),
        }
    }
    pub fn sender_address(&self) -> &Felt {
        match self {
            InvokeTransaction::V0(tx) => &tx.contract_address,
            InvokeTransaction::V1(tx) => &tx.sender_address,
            InvokeTransaction::V3(tx) => &tx.sender_address,
        }
    }

    pub fn signature(&self) -> &[Felt] {
        match self {
            InvokeTransaction::V0(tx) => &tx.signature,
            InvokeTransaction::V1(tx) => &tx.signature,
            InvokeTransaction::V3(tx) => &tx.signature,
        }
    }

    pub fn compute_hash_signature<H: StarkHash>(&self) -> Felt {
        H::hash_array(self.signature())
    }

    pub fn calldata(&self) -> Option<&[Felt]> {
        match self {
            InvokeTransaction::V0(tx) => Some(&tx.calldata),
            InvokeTransaction::V1(tx) => Some(&tx.calldata),
            InvokeTransaction::V3(tx) => Some(&tx.calldata),
        }
    }

    pub fn nonce(&self) -> &Felt {
        match self {
            InvokeTransaction::V0(_) => &Felt::ZERO,
            InvokeTransaction::V1(tx) => &tx.nonce,
            InvokeTransaction::V3(tx) => &tx.nonce,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct InvokeTransactionV0 {
    pub max_fee: Felt,
    pub signature: Vec<Felt>,
    pub contract_address: Felt,
    pub entry_point_selector: Felt,
    pub calldata: Vec<Felt>,
}

impl InvokeTransactionV0 {
    fn version(&self) -> TransactionVersion {
        TransactionVersion::ZERO
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct InvokeTransactionV1 {
    pub sender_address: Felt,
    pub calldata: Vec<Felt>,
    pub max_fee: Felt,
    pub signature: Vec<Felt>,
    pub nonce: Felt,
}

impl InvokeTransactionV1 {
    fn version(&self) -> TransactionVersion {
        TransactionVersion::ONE
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct InvokeTransactionV3 {
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

impl InvokeTransactionV3 {
    fn version(&self) -> TransactionVersion {
        TransactionVersion::THREE
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct L1HandlerTransaction {
    pub version: Felt,
    pub nonce: u64,
    pub contract_address: Felt,
    pub entry_point_selector: Felt,
    pub calldata: Vec<Felt>,
}

impl L1HandlerTransaction {
    fn version(&self) -> TransactionVersion {
        TransactionVersion(self.version)
    }
}

impl From<starknet_core::types::MsgFromL1> for L1HandlerTransaction {
    fn from(msg: starknet_core::types::MsgFromL1) -> Self {
        Self {
            version: Felt::ZERO,
            nonce: 0,
            contract_address: msg.to_address,
            entry_point_selector: msg.entry_point_selector,
            calldata: std::iter::once(msg.from_address.to_felt()).chain(msg.payload).collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum DeclareTransaction {
    V0(DeclareTransactionV0),
    V1(DeclareTransactionV1),
    V2(DeclareTransactionV2),
    V3(DeclareTransactionV3),
}

impl DeclareTransaction {
    fn version(&self) -> TransactionVersion {
        match self {
            DeclareTransaction::V0(tx) => tx.version(),
            DeclareTransaction::V1(tx) => tx.version(),
            DeclareTransaction::V2(tx) => tx.version(),
            DeclareTransaction::V3(tx) => tx.version(),
        }
    }

    pub fn sender_address(&self) -> &Felt {
        match self {
            DeclareTransaction::V0(tx) => &tx.sender_address,
            DeclareTransaction::V1(tx) => &tx.sender_address,
            DeclareTransaction::V2(tx) => &tx.sender_address,
            DeclareTransaction::V3(tx) => &tx.sender_address,
        }
    }
    pub fn signature(&self) -> &[Felt] {
        match self {
            DeclareTransaction::V0(tx) => &tx.signature,
            DeclareTransaction::V1(tx) => &tx.signature,
            DeclareTransaction::V2(tx) => &tx.signature,
            DeclareTransaction::V3(tx) => &tx.signature,
        }
    }

    pub fn compute_hash_signature<H>(&self) -> Felt
    where
        H: StarkHash,
    {
        H::hash_array(self.signature())
    }

    pub fn call_data(&self) -> Option<&[Felt]> {
        None
    }

    pub fn nonce(&self) -> &Felt {
        match self {
            DeclareTransaction::V0(_) => &Felt::ZERO,
            DeclareTransaction::V1(tx) => &tx.nonce,
            DeclareTransaction::V2(tx) => &tx.nonce,
            DeclareTransaction::V3(tx) => &tx.nonce,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DeclareTransactionV0 {
    pub sender_address: Felt,
    pub max_fee: Felt,
    pub signature: Vec<Felt>,
    pub class_hash: Felt,
}

impl DeclareTransactionV0 {
    fn version(&self) -> TransactionVersion {
        TransactionVersion::ZERO
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DeclareTransactionV1 {
    pub sender_address: Felt,
    pub max_fee: Felt,
    pub signature: Vec<Felt>,
    pub nonce: Felt,
    pub class_hash: Felt,
}

impl DeclareTransactionV1 {
    fn version(&self) -> TransactionVersion {
        TransactionVersion::ONE
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DeclareTransactionV2 {
    pub sender_address: Felt,
    pub compiled_class_hash: Felt,
    pub max_fee: Felt,
    pub signature: Vec<Felt>,
    pub nonce: Felt,
    pub class_hash: Felt,
}

impl DeclareTransactionV2 {
    fn version(&self) -> TransactionVersion {
        TransactionVersion::TWO
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DeclareTransactionV3 {
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

impl DeclareTransactionV3 {
    fn version(&self) -> TransactionVersion {
        TransactionVersion::THREE
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DeployTransaction {
    pub version: Felt,
    pub contract_address_salt: Felt,
    pub constructor_calldata: Vec<Felt>,
    pub class_hash: Felt,
}

impl DeployTransaction {
    fn version(&self) -> TransactionVersion {
        TransactionVersion(self.version)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum DeployAccountTransaction {
    V1(DeployAccountTransactionV1),
    V3(DeployAccountTransactionV3),
}

impl DeployAccountTransaction {
    pub fn version(&self) -> TransactionVersion {
        match self {
            DeployAccountTransaction::V1(tx) => tx.version(),
            DeployAccountTransaction::V3(tx) => tx.version(),
        }
    }

    pub fn sender_address(&self) -> &Felt {
        match self {
            DeployAccountTransaction::V1(tx) => &tx.contract_address_salt,
            DeployAccountTransaction::V3(tx) => &tx.contract_address_salt,
        }
    }
    pub fn signature(&self) -> &[Felt] {
        match self {
            DeployAccountTransaction::V1(tx) => &tx.signature,
            DeployAccountTransaction::V3(tx) => &tx.signature,
        }
    }

    pub fn compute_hash_signature<H>(&self) -> Felt
    where
        H: StarkHash,
    {
        H::hash_array(self.signature())
    }

    pub fn calldata(&self) -> Option<&[Felt]> {
        match self {
            DeployAccountTransaction::V1(tx) => Some(&tx.constructor_calldata),
            DeployAccountTransaction::V3(tx) => Some(&tx.constructor_calldata),
        }
    }

    pub fn nonce(&self) -> &Felt {
        match self {
            DeployAccountTransaction::V1(tx) => &tx.nonce,
            DeployAccountTransaction::V3(tx) => &tx.nonce,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DeployAccountTransactionV1 {
    pub max_fee: Felt,
    pub signature: Vec<Felt>,
    pub nonce: Felt,
    pub contract_address_salt: Felt,
    pub constructor_calldata: Vec<Felt>,
    pub class_hash: Felt,
}

impl DeployAccountTransactionV1 {
    fn version(&self) -> TransactionVersion {
        TransactionVersion::ONE
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DeployAccountTransactionV3 {
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

impl DeployAccountTransactionV3 {
    fn version(&self) -> TransactionVersion {
        TransactionVersion::THREE
    }
}

#[derive(Debug, Clone, Default, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum DataAvailabilityMode {
    #[default]
    L1 = 0,
    L2 = 1,
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

impl From<ResourceBoundsMapping> for starknet_core::types::ResourceBoundsMapping {
    fn from(resource: ResourceBoundsMapping) -> Self {
        Self {
            l1_gas: starknet_core::types::ResourceBounds {
                max_amount: resource.l1_gas.max_amount,
                max_price_per_unit: resource.l1_gas.max_price_per_unit,
            },
            l2_gas: starknet_core::types::ResourceBounds {
                max_amount: resource.l2_gas.max_amount,
                max_price_per_unit: resource.l2_gas.max_price_per_unit,
            },
        }
    }
}

impl From<starknet_core::types::ResourceBoundsMapping> for ResourceBoundsMapping {
    fn from(resource: starknet_core::types::ResourceBoundsMapping) -> Self {
        Self {
            l1_gas: ResourceBounds {
                max_amount: resource.l1_gas.max_amount,
                max_price_per_unit: resource.l1_gas.max_price_per_unit,
            },
            l2_gas: ResourceBounds {
                max_amount: resource.l2_gas.max_amount,
                max_price_per_unit: resource.l2_gas.max_price_per_unit,
            },
        }
    }
}

impl From<DataAvailabilityMode> for starknet_core::types::DataAvailabilityMode {
    fn from(da_mode: DataAvailabilityMode) -> Self {
        match da_mode {
            DataAvailabilityMode::L1 => Self::L1,
            DataAvailabilityMode::L2 => Self::L2,
        }
    }
}

impl From<starknet_core::types::DataAvailabilityMode> for DataAvailabilityMode {
    fn from(da_mode: starknet_core::types::DataAvailabilityMode) -> Self {
        match da_mode {
            starknet_core::types::DataAvailabilityMode::L1 => Self::L1,
            starknet_core::types::DataAvailabilityMode::L2 => Self::L2,
        }
    }
}
