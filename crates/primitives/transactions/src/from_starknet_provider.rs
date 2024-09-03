use mp_convert::felt_to_u64;
use starknet_types_core::felt::Felt;

use crate::{
    DataAvailabilityMode, DeclareTransaction, DeclareTransactionV0, DeclareTransactionV1, DeclareTransactionV2,
    DeclareTransactionV3, DeployAccountTransaction, DeployAccountTransactionV1, DeployAccountTransactionV3,
    DeployTransaction, InvokeTransaction, InvokeTransactionV0, InvokeTransactionV1, InvokeTransactionV3,
    L1HandlerTransaction, ResourceBounds, ResourceBoundsMapping, Transaction,
};

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum TransactionTypeError {
    #[error("Invalid version")]
    InvalidVersion,
    #[error("Missing max_fee")]
    MissingMaxFee,
    #[error("Missing compiled_class_hash")]
    MissingCompiledClassHash,
    #[error("Missing tip")]
    MissingTip,
    #[error("Missing resource_bounds")]
    MissingResourceBounds,
    #[error("Missing paymaster_data")]
    MissingPaymasterData,
    #[error("Missing account_deployment_data")]
    MissingAccountDeploymentData,
    #[error("Missing entry_point_selector")]
    MissingEntryPointSelector,
    #[error("Missing nonce")]
    MissingNonce,
    #[error("Missing nonce_data_availability_mode")]
    MissingNonceDataAvailabilityMode,
    #[error("Missing fee_data_availability_mode")]
    MissingFeeDataAvailabilityMode,
    #[error("Invalid nonce")]
    InvalidNonce,
}

impl TryFrom<starknet_providers::sequencer::models::TransactionType> for Transaction {
    type Error = TransactionTypeError;

    fn try_from(value: starknet_providers::sequencer::models::TransactionType) -> Result<Self, Self::Error> {
        match value {
            starknet_providers::sequencer::models::TransactionType::Declare(tx) => Ok(Self::Declare(tx.try_into()?)),
            starknet_providers::sequencer::models::TransactionType::Deploy(tx) => Ok(Self::Deploy(tx.into())),
            starknet_providers::sequencer::models::TransactionType::DeployAccount(tx) => {
                Ok(Self::DeployAccount(tx.try_into()?))
            }
            starknet_providers::sequencer::models::TransactionType::InvokeFunction(tx) => {
                Ok(Self::Invoke(tx.try_into()?))
            }
            starknet_providers::sequencer::models::TransactionType::L1Handler(tx) => {
                Ok(Self::L1Handler(tx.try_into()?))
            }
        }
    }
}

impl TryFrom<starknet_providers::sequencer::models::DeclareTransaction> for DeclareTransaction {
    type Error = TransactionTypeError;

    fn try_from(tx: starknet_providers::sequencer::models::DeclareTransaction) -> Result<Self, Self::Error> {
        let version = tx.version;

        if version == Felt::ZERO {
            Ok(Self::V0(tx.try_into()?))
        } else if version == Felt::ONE {
            Ok(Self::V1(tx.try_into()?))
        } else if version == Felt::TWO {
            Ok(Self::V2(tx.try_into()?))
        } else if version == Felt::THREE {
            Ok(Self::V3(tx.try_into()?))
        } else {
            Err(TransactionTypeError::InvalidVersion)
        }
    }
}

impl TryFrom<starknet_providers::sequencer::models::DeclareTransaction> for DeclareTransactionV0 {
    type Error = TransactionTypeError;

    fn try_from(tx: starknet_providers::sequencer::models::DeclareTransaction) -> Result<Self, Self::Error> {
        Ok(Self {
            sender_address: tx.sender_address,
            max_fee: tx.max_fee.ok_or(TransactionTypeError::MissingMaxFee)?,
            signature: tx.signature,
            class_hash: tx.class_hash,
        })
    }
}

impl TryFrom<starknet_providers::sequencer::models::DeclareTransaction> for DeclareTransactionV1 {
    type Error = TransactionTypeError;

    fn try_from(tx: starknet_providers::sequencer::models::DeclareTransaction) -> Result<Self, Self::Error> {
        Ok(Self {
            sender_address: tx.sender_address,
            max_fee: tx.max_fee.ok_or(TransactionTypeError::MissingMaxFee)?,
            signature: tx.signature,
            nonce: tx.nonce,
            class_hash: tx.class_hash,
        })
    }
}

impl TryFrom<starknet_providers::sequencer::models::DeclareTransaction> for DeclareTransactionV2 {
    type Error = TransactionTypeError;

    fn try_from(tx: starknet_providers::sequencer::models::DeclareTransaction) -> Result<Self, Self::Error> {
        Ok(Self {
            sender_address: tx.sender_address,
            compiled_class_hash: tx.compiled_class_hash.ok_or(TransactionTypeError::MissingCompiledClassHash)?,
            max_fee: tx.max_fee.ok_or(TransactionTypeError::MissingMaxFee)?,
            signature: tx.signature,
            nonce: tx.nonce,
            class_hash: tx.class_hash,
        })
    }
}

impl TryFrom<starknet_providers::sequencer::models::DeclareTransaction> for DeclareTransactionV3 {
    type Error = TransactionTypeError;

    fn try_from(tx: starknet_providers::sequencer::models::DeclareTransaction) -> Result<Self, Self::Error> {
        Ok(Self {
            sender_address: tx.sender_address,
            compiled_class_hash: tx.compiled_class_hash.ok_or(TransactionTypeError::MissingCompiledClassHash)?,
            signature: tx.signature,
            nonce: tx.nonce,
            class_hash: tx.class_hash,
            resource_bounds: tx.resource_bounds.ok_or(TransactionTypeError::MissingResourceBounds)?.into(),
            tip: tx.tip.ok_or(TransactionTypeError::MissingTip)?,
            paymaster_data: tx.paymaster_data.ok_or(TransactionTypeError::MissingPaymasterData)?,
            account_deployment_data: tx
                .account_deployment_data
                .ok_or(TransactionTypeError::MissingAccountDeploymentData)?,
            nonce_data_availability_mode: tx
                .nonce_data_availability_mode
                .ok_or(TransactionTypeError::MissingNonceDataAvailabilityMode)?
                .into(),
            fee_data_availability_mode: tx
                .fee_data_availability_mode
                .ok_or(TransactionTypeError::MissingFeeDataAvailabilityMode)?
                .into(),
        })
    }
}

impl From<starknet_providers::sequencer::models::DeployTransaction> for DeployTransaction {
    fn from(tx: starknet_providers::sequencer::models::DeployTransaction) -> Self {
        Self {
            version: tx.version,
            contract_address_salt: tx.contract_address_salt,
            constructor_calldata: tx.constructor_calldata,
            class_hash: tx.class_hash,
        }
    }
}

impl TryFrom<starknet_providers::sequencer::models::DeployAccountTransaction> for DeployAccountTransaction {
    type Error = TransactionTypeError;

    fn try_from(tx: starknet_providers::sequencer::models::DeployAccountTransaction) -> Result<Self, Self::Error> {
        let version = tx.version;

        if version == Felt::ONE {
            Ok(Self::V1(tx.try_into()?))
        } else if version == Felt::THREE {
            Ok(Self::V3(tx.try_into()?))
        } else {
            Err(TransactionTypeError::InvalidVersion)
        }
    }
}

impl TryFrom<starknet_providers::sequencer::models::DeployAccountTransaction> for DeployAccountTransactionV1 {
    type Error = TransactionTypeError;

    fn try_from(tx: starknet_providers::sequencer::models::DeployAccountTransaction) -> Result<Self, Self::Error> {
        Ok(Self {
            max_fee: tx.max_fee.ok_or(TransactionTypeError::MissingMaxFee)?,
            signature: tx.signature,
            nonce: tx.nonce,
            contract_address_salt: tx.contract_address_salt,
            constructor_calldata: tx.constructor_calldata,
            class_hash: tx.class_hash,
        })
    }
}

impl TryFrom<starknet_providers::sequencer::models::DeployAccountTransaction> for DeployAccountTransactionV3 {
    type Error = TransactionTypeError;

    fn try_from(tx: starknet_providers::sequencer::models::DeployAccountTransaction) -> Result<Self, Self::Error> {
        Ok(Self {
            signature: tx.signature,
            nonce: tx.nonce,
            contract_address_salt: tx.contract_address_salt,
            constructor_calldata: tx.constructor_calldata,
            class_hash: tx.class_hash,
            resource_bounds: tx.resource_bounds.ok_or(TransactionTypeError::MissingResourceBounds)?.into(),
            tip: tx.tip.ok_or(TransactionTypeError::MissingTip)?,
            paymaster_data: tx.paymaster_data.ok_or(TransactionTypeError::MissingPaymasterData)?,
            nonce_data_availability_mode: tx
                .nonce_data_availability_mode
                .ok_or(TransactionTypeError::MissingNonceDataAvailabilityMode)?
                .into(),
            fee_data_availability_mode: tx
                .fee_data_availability_mode
                .ok_or(TransactionTypeError::MissingFeeDataAvailabilityMode)?
                .into(),
        })
    }
}

impl TryFrom<starknet_providers::sequencer::models::InvokeFunctionTransaction> for InvokeTransaction {
    type Error = TransactionTypeError;

    fn try_from(tx: starknet_providers::sequencer::models::InvokeFunctionTransaction) -> Result<Self, Self::Error> {
        let version = tx.version;

        if version == Felt::ZERO {
            Ok(Self::V0(tx.try_into()?))
        } else if version == Felt::ONE {
            Ok(Self::V1(tx.try_into()?))
        } else if version == Felt::THREE {
            Ok(Self::V3(tx.try_into()?))
        } else {
            Err(TransactionTypeError::InvalidVersion)
        }
    }
}

impl TryFrom<starknet_providers::sequencer::models::InvokeFunctionTransaction> for InvokeTransactionV0 {
    type Error = TransactionTypeError;

    fn try_from(tx: starknet_providers::sequencer::models::InvokeFunctionTransaction) -> Result<Self, Self::Error> {
        Ok(Self {
            max_fee: tx.max_fee.ok_or(TransactionTypeError::MissingMaxFee)?,
            signature: tx.signature,
            contract_address: tx.sender_address,
            entry_point_selector: tx.entry_point_selector.ok_or(TransactionTypeError::MissingEntryPointSelector)?,
            calldata: tx.calldata,
        })
    }
}

impl TryFrom<starknet_providers::sequencer::models::InvokeFunctionTransaction> for InvokeTransactionV1 {
    type Error = TransactionTypeError;

    fn try_from(tx: starknet_providers::sequencer::models::InvokeFunctionTransaction) -> Result<Self, Self::Error> {
        Ok(Self {
            sender_address: tx.sender_address,
            calldata: tx.calldata,
            max_fee: tx.max_fee.ok_or(TransactionTypeError::MissingMaxFee)?,
            signature: tx.signature,
            nonce: tx.nonce.ok_or(TransactionTypeError::MissingNonce)?,
        })
    }
}

impl TryFrom<starknet_providers::sequencer::models::InvokeFunctionTransaction> for InvokeTransactionV3 {
    type Error = TransactionTypeError;

    fn try_from(tx: starknet_providers::sequencer::models::InvokeFunctionTransaction) -> Result<Self, Self::Error> {
        Ok(Self {
            sender_address: tx.sender_address,
            calldata: tx.calldata,
            signature: tx.signature,
            nonce: tx.nonce.ok_or(TransactionTypeError::MissingNonce)?,
            resource_bounds: tx.resource_bounds.ok_or(TransactionTypeError::MissingResourceBounds)?.into(),
            tip: tx.tip.ok_or(TransactionTypeError::MissingTip)?,
            paymaster_data: tx.paymaster_data.ok_or(TransactionTypeError::MissingPaymasterData)?,
            account_deployment_data: tx
                .account_deployment_data
                .ok_or(TransactionTypeError::MissingAccountDeploymentData)?,
            nonce_data_availability_mode: tx
                .nonce_data_availability_mode
                .ok_or(TransactionTypeError::MissingNonceDataAvailabilityMode)?
                .into(),
            fee_data_availability_mode: tx
                .fee_data_availability_mode
                .ok_or(TransactionTypeError::MissingFeeDataAvailabilityMode)?
                .into(),
        })
    }
}

impl TryFrom<starknet_providers::sequencer::models::L1HandlerTransaction> for L1HandlerTransaction {
    type Error = TransactionTypeError;

    fn try_from(tx: starknet_providers::sequencer::models::L1HandlerTransaction) -> Result<Self, Self::Error> {
        Ok(Self {
            version: tx.version,
            nonce: felt_to_u64(&tx.nonce.unwrap_or_default()).map_err(|_| TransactionTypeError::InvalidNonce)?,
            contract_address: tx.contract_address,
            entry_point_selector: tx.entry_point_selector,
            calldata: tx.calldata,
        })
    }
}

impl From<starknet_providers::sequencer::models::ResourceBoundsMapping> for ResourceBoundsMapping {
    fn from(resource: starknet_providers::sequencer::models::ResourceBoundsMapping) -> Self {
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

impl From<starknet_providers::sequencer::models::DataAvailabilityMode> for DataAvailabilityMode {
    fn from(da_mode: starknet_providers::sequencer::models::DataAvailabilityMode) -> Self {
        match da_mode {
            starknet_providers::sequencer::models::DataAvailabilityMode::L1 => Self::L1,
            starknet_providers::sequencer::models::DataAvailabilityMode::L2 => Self::L2,
        }
    }
}
