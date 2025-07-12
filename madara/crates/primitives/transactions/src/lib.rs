use std::sync::Arc;

use mp_convert::hex_serde::{U128AsHex, U64AsHex};
use serde::{Deserialize, Serialize};
use serde_with::serde_as;
use starknet_api::{block::FeeType, transaction::TransactionVersion};
use starknet_types_core::{felt::Felt, hash::StarkHash};

mod from_blockifier;
mod from_broadcasted_transaction;
mod from_starknet_types;
mod into_starknet_api;
mod to_blockifier;
mod to_starknet_types;

pub mod compute_hash;
pub mod validated;

pub use to_blockifier::*;

type Signature = Arc<Vec<Felt>>;
type Calldata = Arc<Vec<Felt>>;

const SIMULATE_TX_VERSION_OFFSET: Felt = Felt::from_hex_unchecked("0x100000000000000000000000000000000");

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct L1HandlerTransactionResult {
    /// The hash of the invoke transaction
    pub transaction_hash: Felt,
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

#[derive(Debug, thiserror::Error)]
#[error("Transaction type is note the expected one.")]
pub struct UnexpectedTransactionType;

impl TryFrom<Transaction> for InvokeTransaction {
    type Error = UnexpectedTransactionType;

    fn try_from(tx: Transaction) -> Result<Self, Self::Error> {
        match tx {
            Transaction::Invoke(tx) => Ok(tx),
            _ => Err(UnexpectedTransactionType),
        }
    }
}

impl TryFrom<Transaction> for L1HandlerTransaction {
    type Error = UnexpectedTransactionType;

    fn try_from(tx: Transaction) -> Result<Self, Self::Error> {
        match tx {
            Transaction::L1Handler(tx) => Ok(tx),
            _ => Err(UnexpectedTransactionType),
        }
    }
}

impl TryFrom<Transaction> for DeclareTransaction {
    type Error = UnexpectedTransactionType;

    fn try_from(tx: Transaction) -> Result<Self, Self::Error> {
        match tx {
            Transaction::Declare(tx) => Ok(tx),
            _ => Err(UnexpectedTransactionType),
        }
    }
}

impl TryFrom<Transaction> for DeployTransaction {
    type Error = UnexpectedTransactionType;

    fn try_from(tx: Transaction) -> Result<Self, Self::Error> {
        match tx {
            Transaction::Deploy(tx) => Ok(tx),
            _ => Err(UnexpectedTransactionType),
        }
    }
}

impl TryFrom<Transaction> for DeployAccountTransaction {
    type Error = UnexpectedTransactionType;

    fn try_from(tx: Transaction) -> Result<Self, Self::Error> {
        match tx {
            Transaction::DeployAccount(tx) => Ok(tx),
            _ => Err(UnexpectedTransactionType),
        }
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

    // Note: warning on what this nonce is for l1 handler txs.
    pub fn nonce(&self) -> Felt {
        match self {
            Transaction::Invoke(tx) => *tx.nonce(),
            Transaction::L1Handler(tx) => tx.nonce.into(),
            Transaction::Declare(tx) => *tx.nonce(),
            Transaction::Deploy(_) => Felt::ZERO,
            Transaction::DeployAccount(tx) => *tx.nonce(),
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

    pub fn as_invoke(&self) -> Option<&InvokeTransaction> {
        match self {
            Transaction::Invoke(tx) => Some(tx),
            _ => None,
        }
    }
    pub fn as_declare(&self) -> Option<&DeclareTransaction> {
        match self {
            Transaction::Declare(tx) => Some(tx),
            _ => None,
        }
    }
    pub fn as_l1_handler(&self) -> Option<&L1HandlerTransaction> {
        match self {
            Transaction::L1Handler(tx) => Some(tx),
            _ => None,
        }
    }
    pub fn as_deploy(&self) -> Option<&DeployTransaction> {
        match self {
            Transaction::Deploy(tx) => Some(tx),
            _ => None,
        }
    }
    pub fn as_deploy_account(&self) -> Option<&DeployAccountTransaction> {
        match self {
            Transaction::DeployAccount(tx) => Some(tx),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum InvokeTransaction {
    V0(InvokeTransactionV0),
    V1(InvokeTransactionV1),
    V3(InvokeTransactionV3),
}

impl From<InvokeTransactionV0> for InvokeTransaction {
    fn from(tx: InvokeTransactionV0) -> Self {
        InvokeTransaction::V0(tx)
    }
}

impl From<InvokeTransactionV1> for InvokeTransaction {
    fn from(tx: InvokeTransactionV1) -> Self {
        InvokeTransaction::V1(tx)
    }
}

impl From<InvokeTransactionV3> for InvokeTransaction {
    fn from(tx: InvokeTransactionV3) -> Self {
        InvokeTransaction::V3(tx)
    }
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

    pub fn calldata(&self) -> &[Felt] {
        match self {
            InvokeTransaction::V0(tx) => &tx.calldata,
            InvokeTransaction::V1(tx) => &tx.calldata,
            InvokeTransaction::V3(tx) => &tx.calldata,
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
    pub signature: Signature,
    pub contract_address: Felt,
    pub entry_point_selector: Felt,
    pub calldata: Calldata,
}

impl InvokeTransactionV0 {
    fn version(&self) -> TransactionVersion {
        TransactionVersion::ZERO
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct InvokeTransactionV1 {
    pub sender_address: Felt,
    pub calldata: Calldata,
    pub max_fee: Felt,
    pub signature: Signature,
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
    pub calldata: Calldata,
    pub signature: Signature,
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
    pub calldata: Calldata,
}

impl L1HandlerTransaction {
    fn version(&self) -> TransactionVersion {
        TransactionVersion(self.version)
    }
}

impl From<mp_rpc::MsgFromL1> for L1HandlerTransaction {
    fn from(msg: mp_rpc::MsgFromL1) -> Self {
        Self {
            version: Felt::ZERO,
            nonce: 0,
            contract_address: msg.to_address,
            entry_point_selector: msg.entry_point_selector,
            // TODO: fix type from_address on mp_rpc::MsgFromL1
            calldata: std::iter::once(Felt::from_hex(&msg.from_address).unwrap())
                .chain(msg.payload)
                .collect::<Vec<_>>()
                .into(),
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

impl From<DeclareTransactionV0> for DeclareTransaction {
    fn from(tx: DeclareTransactionV0) -> Self {
        DeclareTransaction::V0(tx)
    }
}

impl From<DeclareTransactionV1> for DeclareTransaction {
    fn from(tx: DeclareTransactionV1) -> Self {
        DeclareTransaction::V1(tx)
    }
}

impl From<DeclareTransactionV2> for DeclareTransaction {
    fn from(tx: DeclareTransactionV2) -> Self {
        DeclareTransaction::V2(tx)
    }
}

impl From<DeclareTransactionV3> for DeclareTransaction {
    fn from(tx: DeclareTransactionV3) -> Self {
        DeclareTransaction::V3(tx)
    }
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
    pub fn class_hash(&self) -> &Felt {
        match self {
            DeclareTransaction::V0(tx) => &tx.class_hash,
            DeclareTransaction::V1(tx) => &tx.class_hash,
            DeclareTransaction::V2(tx) => &tx.class_hash,
            DeclareTransaction::V3(tx) => &tx.class_hash,
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
    pub signature: Signature,
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
    pub signature: Signature,
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
    pub signature: Signature,
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
    pub signature: Signature,
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

impl From<DeployAccountTransactionV1> for DeployAccountTransaction {
    fn from(tx: DeployAccountTransactionV1) -> Self {
        DeployAccountTransaction::V1(tx)
    }
}

impl From<DeployAccountTransactionV3> for DeployAccountTransaction {
    fn from(tx: DeployAccountTransactionV3) -> Self {
        DeployAccountTransaction::V3(tx)
    }
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

    pub fn calldata(&self) -> &[Felt] {
        match self {
            DeployAccountTransaction::V1(tx) => &tx.constructor_calldata,
            DeployAccountTransaction::V3(tx) => &tx.constructor_calldata,
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
    pub signature: Signature,
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
    pub signature: Signature,
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

#[derive(Debug, Clone, Default, Copy, PartialEq, Eq)]
pub enum DataAvailabilityMode {
    #[default]
    L1 = 0,
    L2 = 1,
}

impl serde::Serialize for DataAvailabilityMode {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_u8(*self as u8)
    }
}

impl<'de> serde::Deserialize<'de> for DataAvailabilityMode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = u8::deserialize(deserializer)?;
        match value {
            0 => Ok(DataAvailabilityMode::L1),
            1 => Ok(DataAvailabilityMode::L2),
            _ => Err(serde::de::Error::custom("Invalid value for DataAvailabilityMode")),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub struct ResourceBoundsMapping {
    pub l1_gas: ResourceBounds,
    pub l2_gas: ResourceBounds,
    pub l1_data_gas: ResourceBounds
}

#[serde_as]
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ResourceBounds {
    #[serde_as(as = "U64AsHex")]
    pub max_amount: u64,
    #[serde_as(as = "U128AsHex")]
    pub max_price_per_unit: u128,
}

impl From<ResourceBoundsMapping> for mp_rpc::ResourceBoundsMapping {
    fn from(resource: ResourceBoundsMapping) -> Self {
        Self { l1_gas: resource.l1_gas.into(), l2_gas: resource.l2_gas.into(), l1_data_gas: resource.l1_data_gas.into() }
    }
}

impl From<mp_rpc::ResourceBoundsMapping> for ResourceBoundsMapping {
    fn from(resource: mp_rpc::ResourceBoundsMapping) -> Self {
        Self { l1_gas: resource.l1_gas.into(), l2_gas: resource.l2_gas.into(), l1_data_gas: resource.l1_data_gas.into() }
    }
}

impl From<ResourceBounds> for mp_rpc::ResourceBounds {
    fn from(resource: ResourceBounds) -> Self {
        Self { max_amount: resource.max_amount, max_price_per_unit: resource.max_price_per_unit }
    }
}

impl From<mp_rpc::ResourceBounds> for ResourceBounds {
    fn from(resource: mp_rpc::ResourceBounds) -> Self {
        Self { max_amount: resource.max_amount, max_price_per_unit: resource.max_price_per_unit }
    }
}

impl From<DataAvailabilityMode> for mp_rpc::DaMode {
    fn from(da_mode: DataAvailabilityMode) -> Self {
        match da_mode {
            DataAvailabilityMode::L1 => Self::L1,
            DataAvailabilityMode::L2 => Self::L2,
        }
    }
}

impl From<mp_rpc::DaMode> for DataAvailabilityMode {
    fn from(da_mode: mp_rpc::DaMode) -> Self {
        match da_mode {
            mp_rpc::DaMode::L1 => Self::L1,
            mp_rpc::DaMode::L2 => Self::L2,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tx_with_hash() {
        let tx: Transaction = InvokeTransactionV0::default().into();
        let hash = Felt::from_hex_unchecked("0x1234567890abcdef");
        let tx_with_hash = TransactionWithHash::new(tx.clone(), hash);
        assert_eq!(tx_with_hash.transaction, tx);
        assert_eq!(tx_with_hash.hash, hash);
    }

    #[test]
    fn test_tx_version() {
        let tx: Transaction = InvokeTransactionV0::default().into();
        assert_eq!(tx.version(), TransactionVersion::ZERO);

        let tx: Transaction = InvokeTransactionV1::default().into();
        assert_eq!(tx.version(), TransactionVersion::ONE);

        let tx: Transaction = InvokeTransactionV3::default().into();
        assert_eq!(tx.version(), TransactionVersion::THREE);

        let tx: Transaction = L1HandlerTransaction::default().into();
        assert_eq!(tx.version(), TransactionVersion::ZERO);

        let tx: Transaction = DeclareTransactionV0::default().into();
        assert_eq!(tx.version(), TransactionVersion::ZERO);

        let tx: Transaction = DeclareTransactionV1::default().into();
        assert_eq!(tx.version(), TransactionVersion::ONE);

        let tx: Transaction = DeclareTransactionV2::default().into();
        assert_eq!(tx.version(), TransactionVersion::TWO);

        let tx: Transaction = DeclareTransactionV3::default().into();
        assert_eq!(tx.version(), TransactionVersion::THREE);

        let tx: Transaction = DeployTransaction::default().into();
        assert_eq!(tx.version(), TransactionVersion::ZERO);

        let tx: Transaction = DeployAccountTransactionV1::default().into();
        assert_eq!(tx.version(), TransactionVersion::ONE);

        let tx: Transaction = DeployAccountTransactionV3::default().into();
        assert_eq!(tx.version(), TransactionVersion::THREE);
    }

    #[test]
    fn test_try_from_tx() {
        let invoke_tx: InvokeTransaction = dummy_tx_invoke_v0().into();
        let tx = Transaction::Invoke(invoke_tx.clone());
        assert_eq!(InvokeTransaction::try_from(tx.clone()).unwrap(), invoke_tx);

        let l1_handler_tx: L1HandlerTransaction = dummy_l1_handler();
        let tx = Transaction::L1Handler(l1_handler_tx.clone());
        assert_eq!(L1HandlerTransaction::try_from(tx.clone()).unwrap(), l1_handler_tx);

        let declare_tx: DeclareTransaction = dummy_tx_declare_v0().into();
        let tx = Transaction::Declare(declare_tx.clone());
        assert_eq!(DeclareTransaction::try_from(tx.clone()).unwrap(), declare_tx);

        let deploy_tx: DeployTransaction = dummy_tx_deploy();
        let tx = Transaction::Deploy(deploy_tx.clone());
        assert_eq!(DeployTransaction::try_from(tx.clone()).unwrap(), deploy_tx);

        let deploy_account_tx: DeployAccountTransaction = dummy_tx_deploy_account_v1().into();
        let tx = Transaction::DeployAccount(deploy_account_tx.clone());
        assert_eq!(DeployAccountTransaction::try_from(tx.clone()).unwrap(), deploy_account_tx);
    }

    #[test]
    fn test_tx_is_l1_handler() {
        let tx: Transaction = L1HandlerTransaction::default().into();
        assert!(tx.is_l1_handler());
        let tx: Transaction = InvokeTransactionV0::default().into();
        assert!(!tx.is_l1_handler());
    }

    #[test]
    fn test_tx_is_account() {
        let tx: Transaction = L1HandlerTransaction::default().into();
        assert!(!tx.is_account());

        let tx: Transaction = InvokeTransactionV0::default().into();
        assert!(tx.is_account());
    }

    #[test]
    fn test_tx_fee_type() {
        let tx: Transaction = L1HandlerTransaction::default().into();
        assert!(matches!(tx.fee_type(), FeeType::Eth));

        let tx: Transaction = InvokeTransactionV0::default().into();
        assert!(matches!(tx.fee_type(), FeeType::Eth));

        let tx: Transaction = InvokeTransactionV3::default().into();
        assert!(matches!(tx.fee_type(), FeeType::Strk));
    }

    #[test]
    fn test_sender_address() {
        let tx: InvokeTransaction = dummy_tx_invoke_v0().into();
        assert_eq!(tx.sender_address(), &Felt::from(4));

        let tx: InvokeTransaction = dummy_tx_invoke_v1().into();
        assert_eq!(tx.sender_address(), &Felt::from(1));

        let tx: InvokeTransaction = dummy_tx_invoke_v3().into();
        assert_eq!(tx.sender_address(), &Felt::from(1));

        let tx: DeclareTransaction = dummy_tx_declare_v0().into();
        assert_eq!(tx.sender_address(), &Felt::from(1));

        let tx: DeclareTransaction = dummy_tx_declare_v1().into();
        assert_eq!(tx.sender_address(), &Felt::from(1));

        let tx: DeclareTransaction = dummy_tx_declare_v2().into();
        assert_eq!(tx.sender_address(), &Felt::from(1));

        let tx: DeclareTransaction = dummy_tx_declare_v3().into();
        assert_eq!(tx.sender_address(), &Felt::from(1));

        let tx: DeployAccountTransaction = dummy_tx_deploy_account_v1().into();
        assert_eq!(tx.sender_address(), &Felt::from(5));

        let tx: DeployAccountTransaction = dummy_tx_deploy_account_v3().into();
        assert_eq!(tx.sender_address(), &Felt::from(4));
    }

    #[test]
    fn test_signature() {
        let tx: InvokeTransaction = dummy_tx_invoke_v0().into();
        assert_eq!(tx.signature(), &[Felt::from(2), Felt::from(3)]);

        let tx: InvokeTransaction = dummy_tx_invoke_v1().into();
        assert_eq!(tx.signature(), &[Felt::from(5), Felt::from(6)]);

        let tx: InvokeTransaction = dummy_tx_invoke_v3().into();
        assert_eq!(tx.signature(), &[Felt::from(4), Felt::from(5)]);

        let tx: DeclareTransaction = dummy_tx_declare_v0().into();
        assert_eq!(tx.signature(), &[Felt::from(3), Felt::from(4)]);

        let tx: DeclareTransaction = dummy_tx_declare_v1().into();
        assert_eq!(tx.signature(), &[Felt::from(3), Felt::from(4)]);

        let tx: DeclareTransaction = dummy_tx_declare_v2().into();
        assert_eq!(tx.signature(), &[Felt::from(4), Felt::from(5)]);

        let tx: DeclareTransaction = dummy_tx_declare_v3().into();
        assert_eq!(tx.signature(), &[Felt::from(3), Felt::from(4)]);

        let tx: DeployAccountTransaction = dummy_tx_deploy_account_v1().into();
        assert_eq!(tx.signature(), &[Felt::from(2), Felt::from(3)]);

        let tx: DeployAccountTransaction = dummy_tx_deploy_account_v3().into();
        assert_eq!(tx.signature(), &[Felt::from(1), Felt::from(2)]);
    }

    #[test]
    fn test_compute_hash_signature() {
        let tx: InvokeTransaction = dummy_tx_invoke_v0().into();
        assert_eq!(
            tx.compute_hash_signature::<starknet_types_core::hash::Poseidon>(),
            Felt::from_hex_unchecked("0xcb40676fdafc07998f1391bf66356b5f0c96d5e61bbf5e7ec0aa0320dd8f9b")
        );

        let tx: DeclareTransaction = dummy_tx_declare_v0().into();
        assert_eq!(
            tx.compute_hash_signature::<starknet_types_core::hash::Poseidon>(),
            Felt::from_hex_unchecked("0x22d481b177090ea8db58ceece7d8493e746d690a1708d438c6c4e51b23c81ee")
        );

        let tx: DeployAccountTransaction = dummy_tx_deploy_account_v1().into();
        assert_eq!(
            tx.compute_hash_signature::<starknet_types_core::hash::Poseidon>(),
            Felt::from_hex_unchecked("0xcb40676fdafc07998f1391bf66356b5f0c96d5e61bbf5e7ec0aa0320dd8f9b")
        );
    }

    #[test]
    fn test_calldata() {
        let tx: InvokeTransaction = dummy_tx_invoke_v0().into();
        assert_eq!(tx.calldata(), &[Felt::from(6), Felt::from(7)]);

        let tx: InvokeTransaction = dummy_tx_invoke_v1().into();
        assert_eq!(tx.calldata(), &[Felt::from(2), Felt::from(3)]);

        let tx: InvokeTransaction = dummy_tx_invoke_v3().into();
        assert_eq!(tx.calldata(), &[Felt::from(2), Felt::from(3)]);

        let tx: DeployAccountTransaction = dummy_tx_deploy_account_v1().into();
        assert_eq!(tx.calldata(), &[Felt::from(6), Felt::from(7)]);

        let tx: DeployAccountTransaction = dummy_tx_deploy_account_v3().into();
        assert_eq!(tx.calldata(), &[Felt::from(5), Felt::from(6)]);
    }

    #[test]
    pub fn test_nonce() {
        let tx: InvokeTransaction = dummy_tx_invoke_v0().into();
        assert_eq!(tx.nonce(), &Felt::ZERO);

        let tx: InvokeTransaction = dummy_tx_invoke_v1().into();
        assert_eq!(tx.nonce(), &Felt::from(7));

        let tx: InvokeTransaction = dummy_tx_invoke_v3().into();
        assert_eq!(tx.nonce(), &Felt::from(6));

        let tx: DeclareTransaction = dummy_tx_declare_v0().into();
        assert_eq!(tx.nonce(), &Felt::ZERO);

        let tx: DeclareTransaction = dummy_tx_declare_v1().into();
        assert_eq!(tx.nonce(), &Felt::from(5));

        let tx: DeclareTransaction = dummy_tx_declare_v2().into();
        assert_eq!(tx.nonce(), &Felt::from(6));

        let tx: DeclareTransaction = dummy_tx_declare_v3().into();
        assert_eq!(tx.nonce(), &Felt::from(5));

        let tx: DeployAccountTransaction = dummy_tx_deploy_account_v1().into();
        assert_eq!(tx.nonce(), &Felt::from(4));

        let tx: DeployAccountTransaction = dummy_tx_deploy_account_v3().into();
        assert_eq!(tx.nonce(), &Felt::from(3));
    }

    #[test]
    fn test_msg_to_l1_handler() {
        let msg = mp_rpc::MsgFromL1 {
            from_address: "0x0000000000000000000000000000000000000001".to_string(),
            to_address: Felt::from(2),
            entry_point_selector: Felt::from(3),
            payload: vec![Felt::from(4), Felt::from(5)],
        };

        let l1_handler_expected = L1HandlerTransaction {
            version: Felt::ZERO,
            nonce: 0,
            contract_address: Felt::from(2),
            entry_point_selector: Felt::from(3),
            calldata: vec![Felt::from(1), Felt::from(4), Felt::from(5)].into(),
        };

        assert_eq!(L1HandlerTransaction::from(msg), l1_handler_expected);
    }

    #[test]
    fn test_resource_bounds_mapping_conversion() {
        let resource_mapping = ResourceBoundsMapping {
            l1_gas: ResourceBounds { max_amount: 1, max_price_per_unit: 2 },
            l2_gas: ResourceBounds { max_amount: 3, max_price_per_unit: 4 },
        };

        let starknet_resource_mapping: mp_rpc::ResourceBoundsMapping = resource_mapping.clone().into();
        let resource_mapping_back: ResourceBoundsMapping = starknet_resource_mapping.into();

        assert_eq!(resource_mapping, resource_mapping_back);
    }

    #[test]
    fn test_data_availability_mode_conversion() {
        let da_mode = DataAvailabilityMode::L1;
        let starknet_da_mode: mp_rpc::DaMode = da_mode.into();
        let da_mode_back: DataAvailabilityMode = starknet_da_mode.into();

        assert_eq!(da_mode, da_mode_back);
    }

    pub(crate) fn dummy_tx_invoke_v0() -> InvokeTransactionV0 {
        InvokeTransactionV0 {
            max_fee: Felt::from(1),
            signature: vec![Felt::from(2), Felt::from(3)].into(),
            contract_address: Felt::from(4),
            entry_point_selector: Felt::from(5),
            calldata: vec![Felt::from(6), Felt::from(7)].into(),
        }
    }

    pub(crate) fn dummy_tx_invoke_v1() -> InvokeTransactionV1 {
        InvokeTransactionV1 {
            sender_address: Felt::from(1),
            calldata: vec![Felt::from(2), Felt::from(3)].into(),
            max_fee: Felt::from(4),
            signature: vec![Felt::from(5), Felt::from(6)].into(),
            nonce: Felt::from(7),
        }
    }

    pub(crate) fn dummy_tx_invoke_v3() -> InvokeTransactionV3 {
        InvokeTransactionV3 {
            sender_address: Felt::from(1),
            calldata: vec![Felt::from(2), Felt::from(3)].into(),
            signature: vec![Felt::from(4), Felt::from(5)].into(),
            nonce: Felt::from(6),
            resource_bounds: ResourceBoundsMapping {
                l1_gas: ResourceBounds { max_amount: 1, max_price_per_unit: 2 },
                l2_gas: ResourceBounds { max_amount: 3, max_price_per_unit: 4 },
            },
            tip: 7,
            paymaster_data: vec![Felt::from(8), Felt::from(9)],
            account_deployment_data: vec![Felt::from(10), Felt::from(11)],
            nonce_data_availability_mode: DataAvailabilityMode::L1,
            fee_data_availability_mode: DataAvailabilityMode::L2,
        }
    }

    pub(crate) fn dummy_l1_handler() -> L1HandlerTransaction {
        L1HandlerTransaction {
            version: Felt::from(1),
            nonce: 2,
            contract_address: Felt::from(3),
            entry_point_selector: Felt::from(4),
            calldata: vec![Felt::from(5), Felt::from(6)].into(),
        }
    }

    pub(crate) fn dummy_tx_declare_v0() -> DeclareTransactionV0 {
        DeclareTransactionV0 {
            sender_address: Felt::from(1),
            max_fee: Felt::from(2),
            signature: vec![Felt::from(3), Felt::from(4)].into(),
            class_hash: Felt::from(5),
        }
    }

    pub(crate) fn dummy_tx_declare_v1() -> DeclareTransactionV1 {
        DeclareTransactionV1 {
            sender_address: Felt::from(1),
            max_fee: Felt::from(2),
            signature: vec![Felt::from(3), Felt::from(4)].into(),
            nonce: Felt::from(5),
            class_hash: Felt::from(6),
        }
    }

    pub(crate) fn dummy_tx_declare_v2() -> DeclareTransactionV2 {
        DeclareTransactionV2 {
            sender_address: Felt::from(1),
            compiled_class_hash: Felt::from(2),
            max_fee: Felt::from(3),
            signature: vec![Felt::from(4), Felt::from(5)].into(),
            nonce: Felt::from(6),
            class_hash: Felt::from(7),
        }
    }

    pub(crate) fn dummy_tx_declare_v3() -> DeclareTransactionV3 {
        DeclareTransactionV3 {
            sender_address: Felt::from(1),
            compiled_class_hash: Felt::from(2),
            signature: vec![Felt::from(3), Felt::from(4)].into(),
            nonce: Felt::from(5),
            class_hash: Felt::from(6),
            resource_bounds: ResourceBoundsMapping {
                l1_gas: ResourceBounds { max_amount: 1, max_price_per_unit: 2 },
                l2_gas: ResourceBounds { max_amount: 3, max_price_per_unit: 4 },
            },
            tip: 7,
            paymaster_data: vec![Felt::from(8), Felt::from(9)],
            account_deployment_data: vec![Felt::from(10), Felt::from(11)],
            nonce_data_availability_mode: DataAvailabilityMode::L1,
            fee_data_availability_mode: DataAvailabilityMode::L2,
        }
    }

    pub(crate) fn dummy_tx_deploy() -> DeployTransaction {
        DeployTransaction {
            version: Felt::from(1),
            contract_address_salt: Felt::from(2),
            constructor_calldata: vec![Felt::from(3), Felt::from(4)],
            class_hash: Felt::from(5),
        }
    }

    pub(crate) fn dummy_tx_deploy_account_v1() -> DeployAccountTransactionV1 {
        DeployAccountTransactionV1 {
            max_fee: Felt::from(1),
            signature: vec![Felt::from(2), Felt::from(3)].into(),
            nonce: Felt::from(4),
            contract_address_salt: Felt::from(5),
            constructor_calldata: vec![Felt::from(6), Felt::from(7)],
            class_hash: Felt::from(8),
        }
    }

    pub(crate) fn dummy_tx_deploy_account_v3() -> DeployAccountTransactionV3 {
        DeployAccountTransactionV3 {
            signature: vec![Felt::from(1), Felt::from(2)].into(),
            nonce: Felt::from(3),
            contract_address_salt: Felt::from(4),
            constructor_calldata: vec![Felt::from(5), Felt::from(6)],
            class_hash: Felt::from(7),
            resource_bounds: ResourceBoundsMapping {
                l1_gas: ResourceBounds { max_amount: 1, max_price_per_unit: 2 },
                l2_gas: ResourceBounds { max_amount: 3, max_price_per_unit: 4 },
            },
            tip: 8,
            paymaster_data: vec![Felt::from(9), Felt::from(10)],
            nonce_data_availability_mode: DataAvailabilityMode::L1,
            fee_data_availability_mode: DataAvailabilityMode::L2,
        }
    }
}
