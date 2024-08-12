use std::sync::Arc;

use mp_convert::felt_to_u128;
use starknet_types_core::felt::Felt;

use crate::{
    DataAvailabilityMode, DeclareTransaction, DeclareTransactionV0, DeclareTransactionV1, DeclareTransactionV2,
    DeclareTransactionV3, DeployAccountTransaction, DeployAccountTransactionV1, DeployAccountTransactionV3,
    DeployTransaction, InvokeTransaction, InvokeTransactionV0, InvokeTransactionV1, InvokeTransactionV3,
    L1HandlerTransaction, ResourceBoundsMapping, Transaction,
};

#[derive(Debug, thiserror::Error)]
pub enum TransactionApiError {
    #[error("Invalid contract address")]
    ContractAddress,
    #[error("Invalid max fee")]
    MaxFee,
    #[error("Invalid tip")]
    Tip,
}

impl TryFrom<&Transaction> for starknet_api::transaction::Transaction {
    type Error = TransactionApiError;

    fn try_from(tx: &Transaction) -> Result<Self, Self::Error> {
        match tx {
            Transaction::Invoke(tx) => Ok(Self::Invoke(tx.try_into()?)),
            Transaction::L1Handler(tx) => Ok(Self::L1Handler(tx.try_into()?)),
            Transaction::Declare(tx) => Ok(Self::Declare(tx.try_into()?)),
            Transaction::Deploy(tx) => Ok(Self::Deploy(tx.into())),
            Transaction::DeployAccount(tx) => Ok(Self::DeployAccount(tx.try_into()?)),
        }
    }
}

impl TryFrom<&InvokeTransaction> for starknet_api::transaction::InvokeTransaction {
    type Error = TransactionApiError;

    fn try_from(tx: &InvokeTransaction) -> Result<Self, Self::Error> {
        match tx {
            InvokeTransaction::V0(tx) => Ok(Self::V0(tx.try_into()?)),
            InvokeTransaction::V1(tx) => Ok(Self::V1(tx.try_into()?)),
            InvokeTransaction::V3(tx) => Ok(Self::V3(tx.try_into()?)),
        }
    }
}

impl TryFrom<&InvokeTransactionV0> for starknet_api::transaction::InvokeTransactionV0 {
    type Error = TransactionApiError;

    fn try_from(tx: &InvokeTransactionV0) -> Result<Self, Self::Error> {
        Ok(Self {
            max_fee: fee(&tx.max_fee)?,
            signature: signature(&tx.signature),
            contract_address: contract_address(&tx.contract_address)?,
            entry_point_selector: entry_point_selector(&tx.entry_point_selector),
            calldata: calldata(&tx.calldata),
        })
    }
}

impl TryFrom<&InvokeTransactionV1> for starknet_api::transaction::InvokeTransactionV1 {
    type Error = TransactionApiError;

    fn try_from(tx: &InvokeTransactionV1) -> Result<Self, Self::Error> {
        Ok(Self {
            max_fee: fee(&tx.max_fee)?,
            signature: signature(&tx.signature),
            nonce: nonce(&tx.nonce),
            sender_address: contract_address(&tx.sender_address)?,
            calldata: calldata(&tx.calldata),
        })
    }
}

impl TryFrom<&InvokeTransactionV3> for starknet_api::transaction::InvokeTransactionV3 {
    type Error = TransactionApiError;

    fn try_from(tx: &InvokeTransactionV3) -> Result<Self, Self::Error> {
        Ok(Self {
            resource_bounds: (&tx.resource_bounds).into(),
            tip: tip(tx.tip),
            signature: signature(&tx.signature),
            nonce: nonce(&tx.nonce),
            sender_address: contract_address(&tx.sender_address)?,
            calldata: calldata(&tx.calldata),
            nonce_data_availability_mode: tx.nonce_data_availability_mode.into(),
            fee_data_availability_mode: tx.fee_data_availability_mode.into(),
            paymaster_data: paymaster_data(&tx.paymaster_data),
            account_deployment_data: account_deployment_data(&tx.account_deployment_data),
        })
    }
}

impl TryFrom<&L1HandlerTransaction> for starknet_api::transaction::L1HandlerTransaction {
    type Error = TransactionApiError;

    fn try_from(tx: &L1HandlerTransaction) -> Result<Self, Self::Error> {
        Ok(Self {
            version: version(&tx.version),
            nonce: nonce_u64(tx.nonce),
            contract_address: contract_address(&tx.contract_address)?,
            entry_point_selector: entry_point_selector(&tx.entry_point_selector),
            calldata: calldata(&tx.calldata),
        })
    }
}

impl TryFrom<&DeclareTransaction> for starknet_api::transaction::DeclareTransaction {
    type Error = TransactionApiError;

    fn try_from(tx: &DeclareTransaction) -> Result<Self, Self::Error> {
        match tx {
            DeclareTransaction::V0(tx) => Ok(Self::V0(tx.try_into()?)),
            DeclareTransaction::V1(tx) => Ok(Self::V1(tx.try_into()?)),
            DeclareTransaction::V2(tx) => Ok(Self::V2(tx.try_into()?)),
            DeclareTransaction::V3(tx) => Ok(Self::V3(tx.try_into()?)),
        }
    }
}

impl TryFrom<&DeclareTransactionV0> for starknet_api::transaction::DeclareTransactionV0V1 {
    type Error = TransactionApiError;

    fn try_from(tx: &DeclareTransactionV0) -> Result<Self, Self::Error> {
        Ok(Self {
            max_fee: fee(&tx.max_fee)?,
            signature: signature(&tx.signature),
            nonce: nonce(&Felt::ZERO), // V0 does not have nonce
            class_hash: class_hash(&tx.class_hash),
            sender_address: contract_address(&tx.sender_address)?,
        })
    }
}

impl TryFrom<&DeclareTransactionV1> for starknet_api::transaction::DeclareTransactionV0V1 {
    type Error = TransactionApiError;

    fn try_from(tx: &DeclareTransactionV1) -> Result<Self, Self::Error> {
        Ok(Self {
            max_fee: fee(&tx.max_fee)?,
            signature: signature(&tx.signature),
            nonce: nonce(&tx.nonce),
            class_hash: class_hash(&tx.class_hash),
            sender_address: contract_address(&tx.sender_address)?,
        })
    }
}

impl TryFrom<&DeclareTransactionV2> for starknet_api::transaction::DeclareTransactionV2 {
    type Error = TransactionApiError;

    fn try_from(tx: &DeclareTransactionV2) -> Result<Self, Self::Error> {
        Ok(Self {
            max_fee: fee(&tx.max_fee)?,
            signature: signature(&tx.signature),
            nonce: nonce(&tx.nonce),
            class_hash: class_hash(&tx.class_hash),
            compiled_class_hash: compiled_class_hash(&tx.compiled_class_hash),
            sender_address: contract_address(&tx.sender_address)?,
        })
    }
}

impl TryFrom<&DeclareTransactionV3> for starknet_api::transaction::DeclareTransactionV3 {
    type Error = TransactionApiError;

    fn try_from(tx: &DeclareTransactionV3) -> Result<Self, Self::Error> {
        Ok(Self {
            resource_bounds: (&tx.resource_bounds).into(),
            tip: tip(tx.tip),
            signature: signature(&tx.signature),
            nonce: nonce(&tx.nonce),
            class_hash: class_hash(&tx.class_hash),
            compiled_class_hash: compiled_class_hash(&tx.compiled_class_hash),
            sender_address: contract_address(&tx.sender_address)?,
            nonce_data_availability_mode: tx.nonce_data_availability_mode.into(),
            fee_data_availability_mode: tx.fee_data_availability_mode.into(),
            paymaster_data: paymaster_data(&tx.paymaster_data),
            account_deployment_data: account_deployment_data(&tx.account_deployment_data),
        })
    }
}

impl From<&DeployTransaction> for starknet_api::transaction::DeployTransaction {
    fn from(tx: &DeployTransaction) -> Self {
        Self {
            version: version(&tx.version),
            class_hash: class_hash(&tx.class_hash),
            contract_address_salt: contract_address_salt(&tx.contract_address_salt),
            constructor_calldata: calldata(&tx.constructor_calldata),
        }
    }
}

impl TryFrom<&DeployAccountTransaction> for starknet_api::transaction::DeployAccountTransaction {
    type Error = TransactionApiError;

    fn try_from(tx: &DeployAccountTransaction) -> Result<Self, Self::Error> {
        match tx {
            DeployAccountTransaction::V1(tx) => Ok(Self::V1(tx.try_into()?)),
            DeployAccountTransaction::V3(tx) => Ok(Self::V3(tx.into())),
        }
    }
}

impl TryFrom<&DeployAccountTransactionV1> for starknet_api::transaction::DeployAccountTransactionV1 {
    type Error = TransactionApiError;

    fn try_from(tx: &DeployAccountTransactionV1) -> Result<Self, Self::Error> {
        Ok(Self {
            max_fee: fee(&tx.max_fee)?,
            signature: signature(&tx.signature),
            nonce: nonce(&tx.nonce),
            class_hash: class_hash(&tx.class_hash),
            contract_address_salt: contract_address_salt(&tx.contract_address_salt),
            constructor_calldata: calldata(&tx.constructor_calldata),
        })
    }
}

impl From<&DeployAccountTransactionV3> for starknet_api::transaction::DeployAccountTransactionV3 {
    fn from(tx: &DeployAccountTransactionV3) -> Self {
        Self {
            resource_bounds: (&tx.resource_bounds).into(),
            tip: tip(tx.tip),
            signature: signature(&tx.signature),
            nonce: nonce(&tx.nonce),
            class_hash: class_hash(&tx.class_hash),
            contract_address_salt: contract_address_salt(&tx.contract_address_salt),
            constructor_calldata: calldata(&tx.constructor_calldata),
            nonce_data_availability_mode: tx.nonce_data_availability_mode.into(),
            fee_data_availability_mode: tx.fee_data_availability_mode.into(),
            paymaster_data: paymaster_data(&tx.paymaster_data),
        }
    }
}

fn fee(fee: &Felt) -> Result<starknet_api::transaction::Fee, TransactionApiError> {
    let fee = felt_to_u128(fee).map_err(|_| TransactionApiError::MaxFee)?;
    Ok(starknet_api::transaction::Fee(fee))
}

fn signature(signature: &[Felt]) -> starknet_api::transaction::TransactionSignature {
    starknet_api::transaction::TransactionSignature(signature.to_vec())
}

fn contract_address(contract_address: &Felt) -> Result<starknet_api::core::ContractAddress, TransactionApiError> {
    (*contract_address).try_into().map_err(|_| TransactionApiError::ContractAddress)
}

fn class_hash(class_hash: &Felt) -> starknet_api::core::ClassHash {
    starknet_api::core::ClassHash(*class_hash)
}

fn compiled_class_hash(compiled_class_hash: &Felt) -> starknet_api::core::CompiledClassHash {
    starknet_api::core::CompiledClassHash(*compiled_class_hash)
}

fn entry_point_selector(entry_point_selector: &Felt) -> starknet_api::core::EntryPointSelector {
    starknet_api::core::EntryPointSelector(*entry_point_selector)
}

fn calldata(calldata: &[Felt]) -> starknet_api::transaction::Calldata {
    starknet_api::transaction::Calldata(Arc::new(calldata.to_vec()))
}

fn contract_address_salt(salt: &Felt) -> starknet_api::transaction::ContractAddressSalt {
    starknet_api::transaction::ContractAddressSalt(*salt)
}

fn nonce(nonce: &Felt) -> starknet_api::core::Nonce {
    starknet_api::core::Nonce(*nonce)
}

fn nonce_u64(nonce: u64) -> starknet_api::core::Nonce {
    starknet_api::core::Nonce(nonce.into())
}

fn tip(tip: u64) -> starknet_api::transaction::Tip {
    starknet_api::transaction::Tip(tip)
}

fn paymaster_data(data: &[Felt]) -> starknet_api::transaction::PaymasterData {
    starknet_api::transaction::PaymasterData(data.to_vec())
}

fn account_deployment_data(data: &[Felt]) -> starknet_api::transaction::AccountDeploymentData {
    starknet_api::transaction::AccountDeploymentData(data.to_vec())
}

fn version(version: &Felt) -> starknet_api::transaction::TransactionVersion {
    starknet_api::transaction::TransactionVersion(*version)
}

impl From<&ResourceBoundsMapping> for starknet_api::transaction::ResourceBoundsMapping {
    fn from(resources: &ResourceBoundsMapping) -> Self {
        Self::try_from(vec![
            (
                starknet_api::transaction::Resource::L1Gas,
                starknet_api::transaction::ResourceBounds {
                    max_amount: resources.l1_gas.max_amount,
                    max_price_per_unit: resources.l1_gas.max_price_per_unit,
                },
            ),
            (
                starknet_api::transaction::Resource::L2Gas,
                starknet_api::transaction::ResourceBounds {
                    max_amount: resources.l2_gas.max_amount,
                    max_price_per_unit: resources.l2_gas.max_price_per_unit,
                },
            ),
        ])
        .unwrap()
    }
}

impl From<DataAvailabilityMode> for starknet_api::data_availability::DataAvailabilityMode {
    fn from(mode: DataAvailabilityMode) -> Self {
        match mode {
            DataAvailabilityMode::L1 => Self::L1,
            DataAvailabilityMode::L2 => Self::L2,
        }
    }
}
