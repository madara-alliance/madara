use std::sync::Arc;

use starknet_types_core::felt::Felt;

use crate::{
    DataAvailabilityMode, DeclareTransaction, DeclareTransactionV0, DeclareTransactionV1, DeclareTransactionV2,
    DeclareTransactionV3, DeployAccountTransaction, DeployAccountTransactionV1, DeployAccountTransactionV3,
    DeployTransaction, InvokeTransaction, InvokeTransactionV0, InvokeTransactionV1, InvokeTransactionV3,
    L1HandlerTransaction, ResourceBounds, ResourceBoundsMapping, Transaction,
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

impl From<starknet_api::executable_transaction::AccountTransaction> for Transaction {
    fn from(value: starknet_api::executable_transaction::AccountTransaction) -> Self {
        match value {
            starknet_api::executable_transaction::AccountTransaction::Declare(tx) => Self::Declare(tx.tx.into()),
            starknet_api::executable_transaction::AccountTransaction::DeployAccount(tx) => {
                Self::DeployAccount(tx.tx.into())
            }
            starknet_api::executable_transaction::AccountTransaction::Invoke(tx) => Self::Invoke(tx.tx.into()),
        }
    }
}

impl From<starknet_api::executable_transaction::L1HandlerTransaction> for Transaction {
    fn from(value: starknet_api::executable_transaction::L1HandlerTransaction) -> Self {
        Self::L1Handler(value.tx.into())
    }
}

impl From<starknet_api::transaction::Transaction> for Transaction {
    fn from(value: starknet_api::transaction::Transaction) -> Self {
        match value {
            starknet_api::transaction::Transaction::Declare(tx) => Self::Declare(tx.into()),
            starknet_api::transaction::Transaction::Deploy(tx) => Self::Deploy(tx.into()),
            starknet_api::transaction::Transaction::DeployAccount(tx) => Self::DeployAccount(tx.into()),
            starknet_api::transaction::Transaction::Invoke(tx) => Self::Invoke(tx.into()),
            starknet_api::transaction::Transaction::L1Handler(tx) => Self::L1Handler(tx.into()),
        }
    }
}

impl TryFrom<Transaction> for starknet_api::transaction::Transaction {
    type Error = TransactionApiError;

    fn try_from(tx: Transaction) -> Result<Self, Self::Error> {
        match tx {
            Transaction::Invoke(tx) => Ok(Self::Invoke(tx.try_into()?)),
            Transaction::L1Handler(tx) => Ok(Self::L1Handler(tx.try_into()?)),
            Transaction::Declare(tx) => Ok(Self::Declare(tx.try_into()?)),
            Transaction::Deploy(tx) => Ok(Self::Deploy(tx.into())),
            Transaction::DeployAccount(tx) => Ok(Self::DeployAccount(tx.try_into()?)),
        }
    }
}

impl From<starknet_api::transaction::DeclareTransaction> for DeclareTransaction {
    fn from(value: starknet_api::transaction::DeclareTransaction) -> Self {
        match value {
            starknet_api::transaction::DeclareTransaction::V0(tx) => Self::V0(declare_v0v1_into_v0(tx)),
            starknet_api::transaction::DeclareTransaction::V1(tx) => Self::V1(declare_v0v1_into_v1(tx)),
            starknet_api::transaction::DeclareTransaction::V2(tx) => Self::V2(tx.into()),
            starknet_api::transaction::DeclareTransaction::V3(tx) => Self::V3(tx.into()),
        }
    }
}

impl TryFrom<DeclareTransaction> for starknet_api::transaction::DeclareTransaction {
    type Error = TransactionApiError;

    fn try_from(tx: DeclareTransaction) -> Result<Self, Self::Error> {
        match tx {
            DeclareTransaction::V0(tx) => Ok(Self::V0(tx.try_into()?)),
            DeclareTransaction::V1(tx) => Ok(Self::V1(tx.try_into()?)),
            DeclareTransaction::V2(tx) => Ok(Self::V2(tx.try_into()?)),
            DeclareTransaction::V3(tx) => Ok(Self::V3(tx.try_into()?)),
        }
    }
}

fn declare_v0v1_into_v0(tx: starknet_api::transaction::DeclareTransactionV0V1) -> DeclareTransactionV0 {
    DeclareTransactionV0 {
        sender_address: **tx.sender_address,
        max_fee: tx.max_fee.0.into(),
        signature: tx.signature.0,
        class_hash: *tx.class_hash,
    }
}

impl TryFrom<DeclareTransactionV0> for starknet_api::transaction::DeclareTransactionV0V1 {
    type Error = TransactionApiError;

    fn try_from(tx: DeclareTransactionV0) -> Result<Self, Self::Error> {
        Ok(Self {
            max_fee: fee(&tx.max_fee)?,
            signature: starknet_api::transaction::fields::TransactionSignature(tx.signature),
            nonce: starknet_api::core::Nonce(Felt::ZERO), // V0 does not have nonce
            class_hash: starknet_api::core::ClassHash(tx.class_hash),
            sender_address: contract_address(&tx.sender_address)?,
        })
    }
}

fn declare_v0v1_into_v1(tx: starknet_api::transaction::DeclareTransactionV0V1) -> DeclareTransactionV1 {
    DeclareTransactionV1 {
        sender_address: **tx.sender_address,
        max_fee: tx.max_fee.0.into(),
        signature: tx.signature.0,
        class_hash: *tx.class_hash,
        nonce: *tx.nonce,
    }
}

impl TryFrom<DeclareTransactionV1> for starknet_api::transaction::DeclareTransactionV0V1 {
    type Error = TransactionApiError;

    fn try_from(tx: DeclareTransactionV1) -> Result<Self, Self::Error> {
        Ok(Self {
            max_fee: fee(&tx.max_fee)?,
            signature: starknet_api::transaction::fields::TransactionSignature(tx.signature),
            nonce: starknet_api::core::Nonce(tx.nonce),
            class_hash: starknet_api::core::ClassHash(tx.class_hash),
            sender_address: contract_address(&tx.sender_address)?,
        })
    }
}

impl From<starknet_api::transaction::DeclareTransactionV2> for DeclareTransactionV2 {
    fn from(tx: starknet_api::transaction::DeclareTransactionV2) -> Self {
        Self {
            sender_address: **tx.sender_address,
            max_fee: tx.max_fee.0.into(),
            signature: tx.signature.0,
            class_hash: *tx.class_hash,
            nonce: *tx.nonce,
            compiled_class_hash: tx.compiled_class_hash.0,
        }
    }
}

impl TryFrom<DeclareTransactionV2> for starknet_api::transaction::DeclareTransactionV2 {
    type Error = TransactionApiError;

    fn try_from(tx: DeclareTransactionV2) -> Result<Self, Self::Error> {
        Ok(Self {
            max_fee: fee(&tx.max_fee)?,
            signature: starknet_api::transaction::fields::TransactionSignature(tx.signature),
            nonce: starknet_api::core::Nonce(tx.nonce),
            class_hash: starknet_api::core::ClassHash(tx.class_hash),
            compiled_class_hash: starknet_api::core::CompiledClassHash(tx.compiled_class_hash),
            sender_address: contract_address(&tx.sender_address)?,
        })
    }
}

impl From<starknet_api::transaction::DeclareTransactionV3> for DeclareTransactionV3 {
    fn from(value: starknet_api::transaction::DeclareTransactionV3) -> Self {
        Self {
            sender_address: **value.sender_address,
            signature: value.signature.0,
            class_hash: *value.class_hash,
            nonce: *value.nonce,
            compiled_class_hash: value.compiled_class_hash.0,
            resource_bounds: value.resource_bounds.into(),
            tip: *value.tip,
            paymaster_data: value.paymaster_data.0,
            account_deployment_data: value.account_deployment_data.0,
            nonce_data_availability_mode: value.nonce_data_availability_mode.into(),
            fee_data_availability_mode: value.fee_data_availability_mode.into(),
        }
    }
}

impl TryFrom<DeclareTransactionV3> for starknet_api::transaction::DeclareTransactionV3 {
    type Error = TransactionApiError;

    fn try_from(tx: DeclareTransactionV3) -> Result<Self, Self::Error> {
        Ok(Self {
            resource_bounds: (&tx.resource_bounds).into(),
            tip: starknet_api::transaction::fields::Tip(tx.tip),
            signature: starknet_api::transaction::fields::TransactionSignature(tx.signature),
            nonce: starknet_api::core::Nonce(tx.nonce),
            class_hash: starknet_api::core::ClassHash(tx.class_hash),
            compiled_class_hash: starknet_api::core::CompiledClassHash(tx.compiled_class_hash),
            sender_address: contract_address(&tx.sender_address)?,
            nonce_data_availability_mode: tx.nonce_data_availability_mode.into(),
            fee_data_availability_mode: tx.fee_data_availability_mode.into(),
            paymaster_data: starknet_api::transaction::fields::PaymasterData(tx.paymaster_data),
            account_deployment_data: starknet_api::transaction::fields::AccountDeploymentData(
                tx.account_deployment_data,
            ),
        })
    }
}

impl From<starknet_api::transaction::DeployTransaction> for DeployTransaction {
    fn from(value: starknet_api::transaction::DeployTransaction) -> Self {
        Self {
            version: *value.version,
            contract_address_salt: value.contract_address_salt.0,
            constructor_calldata: value.constructor_calldata.0.to_vec(),
            class_hash: *value.class_hash,
        }
    }
}

impl From<DeployTransaction> for starknet_api::transaction::DeployTransaction {
    fn from(tx: DeployTransaction) -> Self {
        Self {
            version: starknet_api::transaction::TransactionVersion(tx.version),
            class_hash: starknet_api::core::ClassHash(tx.class_hash),
            contract_address_salt: starknet_api::transaction::fields::ContractAddressSalt(tx.contract_address_salt),
            constructor_calldata: calldata(tx.constructor_calldata),
        }
    }
}

impl From<starknet_api::transaction::DeployAccountTransaction> for DeployAccountTransaction {
    fn from(value: starknet_api::transaction::DeployAccountTransaction) -> Self {
        match value {
            starknet_api::transaction::DeployAccountTransaction::V1(tx) => Self::V1(tx.into()),
            starknet_api::transaction::DeployAccountTransaction::V3(tx) => Self::V3(tx.into()),
        }
    }
}

impl TryFrom<DeployAccountTransaction> for starknet_api::transaction::DeployAccountTransaction {
    type Error = TransactionApiError;

    fn try_from(tx: DeployAccountTransaction) -> Result<Self, Self::Error> {
        match tx {
            DeployAccountTransaction::V1(tx) => Ok(Self::V1(tx.try_into()?)),
            DeployAccountTransaction::V3(tx) => Ok(Self::V3(tx.into())),
        }
    }
}

impl From<starknet_api::transaction::DeployAccountTransactionV1> for DeployAccountTransactionV1 {
    fn from(value: starknet_api::transaction::DeployAccountTransactionV1) -> Self {
        Self {
            max_fee: value.max_fee.into(),
            signature: value.signature.0,
            nonce: *value.nonce,
            contract_address_salt: value.contract_address_salt.0,
            constructor_calldata: value.constructor_calldata.0.to_vec(),
            class_hash: *value.class_hash,
        }
    }
}

impl TryFrom<DeployAccountTransactionV1> for starknet_api::transaction::DeployAccountTransactionV1 {
    type Error = TransactionApiError;

    fn try_from(tx: DeployAccountTransactionV1) -> Result<Self, Self::Error> {
        Ok(Self {
            max_fee: fee(&tx.max_fee)?,
            signature: starknet_api::transaction::fields::TransactionSignature(tx.signature),
            nonce: starknet_api::core::Nonce(tx.nonce),
            class_hash: starknet_api::core::ClassHash(tx.class_hash),
            contract_address_salt: starknet_api::transaction::fields::ContractAddressSalt(tx.contract_address_salt),
            constructor_calldata: calldata(tx.constructor_calldata),
        })
    }
}

impl From<starknet_api::transaction::DeployAccountTransactionV3> for DeployAccountTransactionV3 {
    fn from(value: starknet_api::transaction::DeployAccountTransactionV3) -> Self {
        Self {
            signature: value.signature.0,
            nonce: *value.nonce,
            contract_address_salt: value.contract_address_salt.0,
            constructor_calldata: value.constructor_calldata.0.to_vec(),
            class_hash: *value.class_hash,
            resource_bounds: value.resource_bounds.into(),
            tip: *value.tip,
            paymaster_data: value.paymaster_data.0,
            nonce_data_availability_mode: value.nonce_data_availability_mode.into(),
            fee_data_availability_mode: value.fee_data_availability_mode.into(),
        }
    }
}

impl From<starknet_api::transaction::InvokeTransaction> for InvokeTransaction {
    fn from(value: starknet_api::transaction::InvokeTransaction) -> Self {
        match value {
            starknet_api::transaction::InvokeTransaction::V0(tx) => Self::V0(tx.into()),
            starknet_api::transaction::InvokeTransaction::V1(tx) => Self::V1(tx.into()),
            starknet_api::transaction::InvokeTransaction::V3(tx) => Self::V3(tx.into()),
        }
    }
}

impl From<DeployAccountTransactionV3> for starknet_api::transaction::DeployAccountTransactionV3 {
    fn from(tx: DeployAccountTransactionV3) -> Self {
        Self {
            resource_bounds: (&tx.resource_bounds).into(),
            tip: starknet_api::transaction::fields::Tip(tx.tip),
            signature: starknet_api::transaction::fields::TransactionSignature(tx.signature),
            nonce: starknet_api::core::Nonce(tx.nonce),
            class_hash: starknet_api::core::ClassHash(tx.class_hash),
            contract_address_salt: starknet_api::transaction::fields::ContractAddressSalt(tx.contract_address_salt),
            constructor_calldata: calldata(tx.constructor_calldata),
            nonce_data_availability_mode: tx.nonce_data_availability_mode.into(),
            fee_data_availability_mode: tx.fee_data_availability_mode.into(),
            paymaster_data: starknet_api::transaction::fields::PaymasterData(tx.paymaster_data),
        }
    }
}

impl TryFrom<InvokeTransaction> for starknet_api::transaction::InvokeTransaction {
    type Error = TransactionApiError;

    fn try_from(tx: InvokeTransaction) -> Result<Self, Self::Error> {
        match tx {
            InvokeTransaction::V0(tx) => Ok(Self::V0(tx.try_into()?)),
            InvokeTransaction::V1(tx) => Ok(Self::V1(tx.try_into()?)),
            InvokeTransaction::V3(tx) => Ok(Self::V3(tx.try_into()?)),
        }
    }
}

impl From<starknet_api::transaction::InvokeTransactionV0> for InvokeTransactionV0 {
    fn from(value: starknet_api::transaction::InvokeTransactionV0) -> Self {
        Self {
            max_fee: value.max_fee.into(),
            signature: value.signature.0,
            contract_address: **value.contract_address,
            entry_point_selector: value.entry_point_selector.0,
            calldata: value.calldata.0.to_vec(),
        }
    }
}

impl TryFrom<InvokeTransactionV0> for starknet_api::transaction::InvokeTransactionV0 {
    type Error = TransactionApiError;

    fn try_from(tx: InvokeTransactionV0) -> Result<Self, Self::Error> {
        Ok(Self {
            max_fee: fee(&tx.max_fee)?,
            signature: starknet_api::transaction::fields::TransactionSignature(tx.signature),
            contract_address: contract_address(&tx.contract_address)?,
            entry_point_selector: starknet_api::core::EntryPointSelector(tx.entry_point_selector),
            calldata: calldata(tx.calldata),
        })
    }
}

impl From<starknet_api::transaction::InvokeTransactionV1> for InvokeTransactionV1 {
    fn from(value: starknet_api::transaction::InvokeTransactionV1) -> Self {
        Self {
            sender_address: **value.sender_address,
            calldata: value.calldata.0.to_vec(),
            max_fee: value.max_fee.into(),
            signature: value.signature.0,
            nonce: *value.nonce,
        }
    }
}

impl TryFrom<InvokeTransactionV1> for starknet_api::transaction::InvokeTransactionV1 {
    type Error = TransactionApiError;

    fn try_from(tx: InvokeTransactionV1) -> Result<Self, Self::Error> {
        Ok(Self {
            max_fee: fee(&tx.max_fee)?,
            signature: starknet_api::transaction::fields::TransactionSignature(tx.signature),
            nonce: starknet_api::core::Nonce(tx.nonce),
            sender_address: contract_address(&tx.sender_address)?,
            calldata: calldata(tx.calldata),
        })
    }
}

impl From<starknet_api::transaction::InvokeTransactionV3> for InvokeTransactionV3 {
    fn from(value: starknet_api::transaction::InvokeTransactionV3) -> Self {
        Self {
            sender_address: **value.sender_address,
            calldata: value.calldata.0.to_vec(),
            signature: value.signature.0,
            nonce: *value.nonce,
            resource_bounds: value.resource_bounds.into(),
            tip: *value.tip,
            paymaster_data: value.paymaster_data.0,
            account_deployment_data: value.account_deployment_data.0,
            nonce_data_availability_mode: value.nonce_data_availability_mode.into(),
            fee_data_availability_mode: value.fee_data_availability_mode.into(),
        }
    }
}

impl TryFrom<InvokeTransactionV3> for starknet_api::transaction::InvokeTransactionV3 {
    type Error = TransactionApiError;

    fn try_from(tx: InvokeTransactionV3) -> Result<Self, Self::Error> {
        Ok(Self {
            resource_bounds: (&tx.resource_bounds).into(),
            tip: starknet_api::transaction::fields::Tip(tx.tip),
            signature: starknet_api::transaction::fields::TransactionSignature(tx.signature),
            nonce: starknet_api::core::Nonce(tx.nonce),
            sender_address: contract_address(&tx.sender_address)?,
            calldata: calldata(tx.calldata),
            nonce_data_availability_mode: tx.nonce_data_availability_mode.into(),
            fee_data_availability_mode: tx.fee_data_availability_mode.into(),
            paymaster_data: starknet_api::transaction::fields::PaymasterData(tx.paymaster_data),
            account_deployment_data: starknet_api::transaction::fields::AccountDeploymentData(
                tx.account_deployment_data,
            ),
        })
    }
}

impl From<starknet_api::transaction::L1HandlerTransaction> for L1HandlerTransaction {
    fn from(value: starknet_api::transaction::L1HandlerTransaction) -> Self {
        Self {
            version: value.version.0,
            nonce: value.nonce.0.try_into().unwrap_or_default(),
            contract_address: **value.contract_address,
            entry_point_selector: value.entry_point_selector.0,
            calldata: value.calldata.0.to_vec(),
        }
    }
}

impl TryFrom<L1HandlerTransaction> for starknet_api::transaction::L1HandlerTransaction {
    type Error = TransactionApiError;

    fn try_from(tx: L1HandlerTransaction) -> Result<Self, Self::Error> {
        Ok(Self {
            version: starknet_api::transaction::TransactionVersion(tx.version),
            nonce: starknet_api::core::Nonce(tx.nonce.into()),
            contract_address: contract_address(&tx.contract_address)?,
            entry_point_selector: starknet_api::core::EntryPointSelector(tx.entry_point_selector),
            calldata: calldata(tx.calldata),
        })
    }
}

impl From<starknet_api::data_availability::DataAvailabilityMode> for DataAvailabilityMode {
    fn from(value: starknet_api::data_availability::DataAvailabilityMode) -> Self {
        match value {
            starknet_api::data_availability::DataAvailabilityMode::L1 => Self::L1,
            starknet_api::data_availability::DataAvailabilityMode::L2 => Self::L2,
        }
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

impl From<starknet_api::transaction::fields::ValidResourceBounds> for ResourceBoundsMapping {
    fn from(value: starknet_api::transaction::fields::ValidResourceBounds) -> Self {
        ResourceBoundsMapping { l1_gas: value.get_l1_bounds().into(), l2_gas: value.get_l2_bounds().into() }
    }
}

impl From<&ResourceBoundsMapping> for starknet_api::transaction::fields::ValidResourceBounds {
    fn from(resources: &ResourceBoundsMapping) -> Self {
        starknet_api::transaction::fields::ValidResourceBounds::L1Gas(
            starknet_api::transaction::fields::ResourceBounds {
                max_amount: resources.l1_gas.max_amount.into(),
                max_price_per_unit: resources.l1_gas.max_price_per_unit.into(),
            },
        )
    }
}

impl From<starknet_api::transaction::fields::ResourceBounds> for ResourceBounds {
    fn from(value: starknet_api::transaction::fields::ResourceBounds) -> Self {
        ResourceBounds { max_amount: value.max_amount.0, max_price_per_unit: value.max_price_per_unit.0 }
    }
}

fn fee(fee: &Felt) -> Result<starknet_api::transaction::fields::Fee, TransactionApiError> {
    let fee = (*fee).try_into().map_err(|_| TransactionApiError::MaxFee)?;
    Ok(starknet_api::transaction::fields::Fee(fee))
}

fn contract_address(contract_address: &Felt) -> Result<starknet_api::core::ContractAddress, TransactionApiError> {
    (*contract_address).try_into().map_err(|_| TransactionApiError::ContractAddress)
}

fn calldata(calldata: Vec<Felt>) -> starknet_api::transaction::fields::Calldata {
    starknet_api::transaction::fields::Calldata(Arc::new(calldata))
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::tests::{
        dummy_l1_handler, dummy_tx_declare_v0, dummy_tx_declare_v1, dummy_tx_declare_v2, dummy_tx_declare_v3,
        dummy_tx_deploy, dummy_tx_deploy_account_v1, dummy_tx_deploy_account_v3, dummy_tx_invoke_v0,
        dummy_tx_invoke_v1, dummy_tx_invoke_v3,
    };
    use mp_convert::test::assert_consistent_conversion;

    #[test]
    fn test_into_api_transaction() {
        let tx: Transaction = dummy_tx_invoke_v0().into();
        assert_consistent_conversion::<_, starknet_api::transaction::Transaction>(tx);

        let tx: Transaction = dummy_tx_invoke_v1().into();
        assert_consistent_conversion::<_, starknet_api::transaction::Transaction>(tx);

        let tx: Transaction = dummy_tx_invoke_v3().into();
        assert_consistent_conversion::<_, starknet_api::transaction::Transaction>(tx);

        let tx: Transaction = dummy_l1_handler().into();
        assert_consistent_conversion::<_, starknet_api::transaction::Transaction>(tx);

        let tx: Transaction = dummy_tx_declare_v0().into();
        assert_consistent_conversion::<_, starknet_api::transaction::Transaction>(tx);

        let tx: Transaction = dummy_tx_declare_v1().into();
        assert_consistent_conversion::<_, starknet_api::transaction::Transaction>(tx);

        let tx: Transaction = dummy_tx_declare_v2().into();
        assert_consistent_conversion::<_, starknet_api::transaction::Transaction>(tx);

        let tx: Transaction = dummy_tx_declare_v3().into();
        assert_consistent_conversion::<_, starknet_api::transaction::Transaction>(tx);

        let tx: Transaction = dummy_tx_deploy().into();
        assert_consistent_conversion::<_, starknet_api::transaction::Transaction>(tx);

        let tx: Transaction = dummy_tx_deploy_account_v1().into();
        assert_consistent_conversion::<_, starknet_api::transaction::Transaction>(tx);

        let tx: Transaction = dummy_tx_deploy_account_v3().into();
        assert_consistent_conversion::<_, starknet_api::transaction::Transaction>(tx);
    }
}
