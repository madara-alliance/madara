use crate::FromModelError;
use crate::{
    model::{self, receipt::execution_resources::BuiltinCounter},
    model_field, model_field_variant, CollectInto, TryIntoField,
};
use m_proc_macros::model_describe;
use mp_block::TransactionWithReceipt;
use mp_receipt::{
    DeclareTransactionReceipt, DeployAccountTransactionReceipt, DeployTransactionReceipt, ExecutionResources,
    ExecutionResult, FeePayment, InvokeTransactionReceipt, L1Gas, L1HandlerTransactionReceipt, MsgToL1, PriceUnit,
    TransactionReceipt,
};
use mp_transactions::{
    DataAvailabilityMode, DeclareTransaction, DeclareTransactionV0, DeclareTransactionV1, DeclareTransactionV2,
    DeclareTransactionV3, DeployAccountTransaction, DeployAccountTransactionV1, DeployAccountTransactionV3,
    DeployTransaction, InvokeTransaction, InvokeTransactionV0, InvokeTransactionV1, InvokeTransactionV3,
    L1HandlerTransaction, ResourceBounds, ResourceBoundsMapping, Transaction, TransactionWithHash,
};
use starknet_core::types::Felt;

impl From<TransactionWithReceipt> for model::TransactionWithReceipt {
    fn from(value: TransactionWithReceipt) -> Self {
        Self {
            transaction: Some(model::TransactionInBlock {
                transaction_hash: Some(value.receipt.transaction_hash().into()),
                txn: Some(value.transaction.into()),
            }),
            receipt: Some(value.receipt.into()),
        }
    }
}

impl TryFrom<model::TransactionWithReceipt> for TransactionWithReceipt {
    type Error = FromModelError;

    #[model_describe(model::TransactionWithReceipt)]
    fn try_from(value: model::TransactionWithReceipt) -> Result<Self, Self::Error> {
        let transaction = model_field!(value => transaction);
        let TransactionWithHash { transaction, hash } = TransactionWithHash::try_from(transaction)?;
        let receipt = model_field!(value => receipt).parse_model(hash)?;
        Ok(Self { transaction, receipt })
    }
}

impl TryFrom<model::TransactionInBlock> for TransactionWithHash {
    type Error = FromModelError;

    #[model_describe(model::TransactionInBlock)]
    fn try_from(value: model::TransactionInBlock) -> Result<Self, Self::Error> {
        Ok(Self {
            transaction: model_field!(value => txn).try_into()?,
            hash: model_field!(value => transaction_hash).into(),
        })
    }
}

impl TryFrom<model::transaction_in_block::Txn> for Transaction {
    type Error = FromModelError;
    fn try_from(value: model::transaction_in_block::Txn) -> Result<Self, Self::Error> {
        use model::transaction_in_block::Txn;
        Ok(match value {
            Txn::DeclareV0(tx) => Self::Declare(DeclareTransaction::V0(tx.try_into()?)),
            Txn::DeclareV1(tx) => Self::Declare(DeclareTransaction::V1(tx.try_into()?)),
            Txn::DeclareV2(tx) => Self::Declare(DeclareTransaction::V2(tx.try_into()?)),
            Txn::DeclareV3(tx) => Self::Declare(DeclareTransaction::V3(tx.try_into()?)),
            Txn::Deploy(tx) => Self::Deploy(tx.try_into()?),
            Txn::DeployAccountV1(tx) => Self::DeployAccount(DeployAccountTransaction::V1(tx.try_into()?)),
            Txn::DeployAccountV3(tx) => Self::DeployAccount(DeployAccountTransaction::V3(tx.try_into()?)),
            Txn::InvokeV0(tx) => Self::Invoke(InvokeTransaction::V0(tx.try_into()?)),
            Txn::InvokeV1(tx) => Self::Invoke(InvokeTransaction::V1(tx.try_into()?)),
            Txn::InvokeV3(tx) => Self::Invoke(InvokeTransaction::V3(tx.try_into()?)),
            Txn::L1Handler(tx) => Self::L1Handler(tx.try_into()?),
        })
    }
}

impl TryFrom<model::transaction_in_block::DeclareV0WithoutClass> for DeclareTransactionV0 {
    type Error = FromModelError;

    #[model_describe(model::transaction_in_block::DeclareV0WithoutClass)]
    fn try_from(value: model::transaction_in_block::DeclareV0WithoutClass) -> Result<Self, Self::Error> {
        Ok(Self {
            sender_address: model_field!(value => sender).into(),
            max_fee: model_field!(value => max_fee).into(),
            signature: model_field!(value => signature).parts.collect_into(),
            class_hash: model_field!(value => class_hash).into(),
        })
    }
}

impl TryFrom<model::transaction_in_block::DeclareV1WithoutClass> for DeclareTransactionV1 {
    type Error = FromModelError;

    #[model_describe(model::transaction_in_block::DeclareV1WithoutClass)]
    fn try_from(value: model::transaction_in_block::DeclareV1WithoutClass) -> Result<Self, Self::Error> {
        Ok(Self {
            sender_address: model_field!(value => sender).into(),
            max_fee: model_field!(value => max_fee).into(),
            signature: model_field!(value => signature).parts.collect_into(),
            nonce: model_field!(value => nonce).into(),
            class_hash: model_field!(value => class_hash).into(),
        })
    }
}

impl TryFrom<model::transaction_in_block::DeclareV2WithoutClass> for DeclareTransactionV2 {
    type Error = FromModelError;

    #[model_describe(model::transaction_in_block::DeclareV2WithoutClass)]
    fn try_from(value: model::transaction_in_block::DeclareV2WithoutClass) -> Result<Self, Self::Error> {
        Ok(Self {
            sender_address: model_field!(value => sender).into(),
            compiled_class_hash: model_field!(value => compiled_class_hash).into(),
            max_fee: model_field!(value => max_fee).into(),
            signature: model_field!(value => signature).parts.collect_into(),
            nonce: model_field!(value => nonce).into(),
            class_hash: model_field!(value => class_hash).into(),
        })
    }
}

impl TryFrom<model::transaction_in_block::DeclareV3WithoutClass> for DeclareTransactionV3 {
    type Error = FromModelError;

    #[model_describe(model::transaction_in_block::DeclareV3WithoutClass)]
    fn try_from(value: model::transaction_in_block::DeclareV3WithoutClass) -> Result<Self, Self::Error> {
        let common = model_field!(value => common);
        Ok(Self {
            sender_address: model_field!(common => sender).into(),
            compiled_class_hash: model_field!(common => compiled_class_hash).into(),
            signature: model_field!(common => signature).parts.collect_into(),
            nonce: model_field!(common => nonce).into(),
            class_hash: model_field!(value => class_hash).into(),
            resource_bounds: model_field!(common => resource_bounds).try_into()?,
            tip: common.tip,
            paymaster_data: common.paymaster_data.collect_into(),
            account_deployment_data: common.account_deployment_data.collect_into(),
            nonce_data_availability_mode:
                model_field_variant!(model::VolitionDomain => common.nonce_data_availability_mode).into(),
            fee_data_availability_mode:
                model_field_variant!(model::VolitionDomain => common.fee_data_availability_mode).into(),
        })
    }
}

impl TryFrom<model::transaction_in_block::Deploy> for DeployTransaction {
    type Error = FromModelError;

    #[model_describe(model::transaction_in_block::Deploy)]
    fn try_from(value: model::transaction_in_block::Deploy) -> Result<Self, Self::Error> {
        Ok(Self {
            version: value.version.into(),
            contract_address_salt: model_field!(value => address_salt).into(),
            constructor_calldata: value.calldata.collect_into(),
            class_hash: model_field!(value => class_hash).into(),
        })
    }
}

impl TryFrom<model::transaction_in_block::DeployAccountV1> for DeployAccountTransactionV1 {
    type Error = FromModelError;

    #[model_describe(model::transaction_in_block::DeployAccountV1)]
    fn try_from(value: model::transaction_in_block::DeployAccountV1) -> Result<Self, Self::Error> {
        Ok(Self {
            max_fee: model_field!(value => max_fee).into(),
            signature: model_field!(value => signature).parts.collect_into(),
            nonce: model_field!(value => nonce).into(),
            contract_address_salt: model_field!(value => address_salt).into(),
            constructor_calldata: value.calldata.collect_into(),
            class_hash: model_field!(value => class_hash).into(),
        })
    }
}

impl TryFrom<model::DeployAccountV3> for DeployAccountTransactionV3 {
    type Error = FromModelError;

    #[model_describe(model::DeployAccountV3)]
    fn try_from(value: model::DeployAccountV3) -> Result<Self, Self::Error> {
        Ok(Self {
            signature: model_field!(value => signature).parts.collect_into(),
            nonce: model_field!(value => nonce).into(),
            contract_address_salt: model_field!(value => address_salt).into(),
            constructor_calldata: value.calldata.collect_into(),
            class_hash: model_field!(value => class_hash).into(),
            resource_bounds: model_field!(value => resource_bounds).try_into()?,
            tip: value.tip,
            paymaster_data: value.paymaster_data.collect_into(),
            nonce_data_availability_mode:
                model_field_variant!(model::VolitionDomain => value.nonce_data_availability_mode).into(),
            fee_data_availability_mode: model_field_variant!(model::VolitionDomain => value.fee_data_availability_mode)
                .into(),
        })
    }
}

impl TryFrom<model::transaction_in_block::InvokeV0> for InvokeTransactionV0 {
    type Error = FromModelError;

    #[model_describe(model::transaction_in_block::InvokeV0)]
    fn try_from(value: model::transaction_in_block::InvokeV0) -> Result<Self, Self::Error> {
        Ok(Self {
            max_fee: model_field!(value => max_fee).into(),
            signature: model_field!(value => signature).parts.collect_into(),
            contract_address: model_field!(value => address).into(),
            entry_point_selector: model_field!(value => entry_point_selector).into(),
            calldata: value.calldata.collect_into(),
        })
    }
}

impl TryFrom<model::transaction_in_block::InvokeV1> for InvokeTransactionV1 {
    type Error = FromModelError;

    #[model_describe(model::transaction_in_block::InvokeV1)]
    fn try_from(value: model::transaction_in_block::InvokeV1) -> Result<Self, Self::Error> {
        Ok(Self {
            sender_address: model_field!(value => sender).into(),
            calldata: value.calldata.collect_into(),
            max_fee: model_field!(value => max_fee).into(),
            signature: model_field!(value => signature).parts.collect_into(),
            nonce: model_field!(value => nonce).into(),
        })
    }
}

impl TryFrom<model::InvokeV3> for InvokeTransactionV3 {
    type Error = FromModelError;

    #[model_describe(model::InvokeV3)]
    fn try_from(value: model::InvokeV3) -> Result<Self, Self::Error> {
        Ok(Self {
            sender_address: model_field!(value => sender).into(),
            calldata: value.calldata.collect_into(),
            signature: model_field!(value => signature).parts.collect_into(),
            nonce: model_field!(value => nonce).into(),
            resource_bounds: model_field!(value => resource_bounds).try_into()?,
            tip: value.tip,
            paymaster_data: value.paymaster_data.collect_into(),
            account_deployment_data: value.account_deployment_data.collect_into(),
            nonce_data_availability_mode:
                model_field_variant!(model::VolitionDomain => value.nonce_data_availability_mode).into(),
            fee_data_availability_mode: model_field_variant!(model::VolitionDomain => value.fee_data_availability_mode)
                .into(),
        })
    }
}

impl TryFrom<model::L1HandlerV0> for L1HandlerTransaction {
    type Error = FromModelError;

    #[model_describe(model::L1HandlerV0)]
    fn try_from(value: model::L1HandlerV0) -> Result<Self, Self::Error> {
        Ok(Self {
            version: Felt::ZERO,
            nonce: model_field!(value => nonce).0.try_into_field("L1HandlerV0::nonce")?,
            contract_address: model_field!(value => address).into(),
            entry_point_selector: model_field!(value => entry_point_selector).into(),
            calldata: value.calldata.collect_into(),
        })
    }
}

impl TryFrom<model::ResourceBounds> for ResourceBoundsMapping {
    type Error = FromModelError;

    #[model_describe(model::ResourceBounds)]
    fn try_from(value: model::ResourceBounds) -> Result<Self, Self::Error> {
        Ok(Self {
            l1_gas: model_field!(value => l1_gas).try_into()?,
            l2_gas: model_field!(value => l2_gas).try_into()?,
        })
    }
}

impl TryFrom<model::ResourceLimits> for ResourceBounds {
    type Error = FromModelError;

    #[model_describe(model::ResourceLimits)]
    fn try_from(value: model::ResourceLimits) -> Result<Self, Self::Error> {
        Ok(Self {
            max_amount: model_field!(value => max_amount).0.try_into_field("ResourceLimits::max_amount")?,
            max_price_per_unit: model_field!(value => max_price_per_unit)
                .0
                .try_into_field("ResourceLimits::max_price_per_unit")?,
        })
    }
}

impl From<model::VolitionDomain> for DataAvailabilityMode {
    fn from(value: model::VolitionDomain) -> Self {
        use model::VolitionDomain;
        match value {
            VolitionDomain::L1 => DataAvailabilityMode::L1,
            VolitionDomain::L2 => DataAvailabilityMode::L2,
        }
    }
}

fn execution_result(revert_reason: Option<String>) -> ExecutionResult {
    match revert_reason {
        Some(reason) => ExecutionResult::Reverted { reason },
        None => ExecutionResult::Succeeded,
    }
}

impl model::Receipt {
    #[model_describe(model::Receipt)]
    pub fn parse_model(self, transaction_hash: Felt) -> Result<TransactionReceipt, FromModelError> {
        use model::receipt::Type;

        Ok(match model_field!(self => r#type) {
            Type::Invoke(tx) => TransactionReceipt::Invoke(tx.parse_model(transaction_hash)?),
            Type::L1Handler(tx) => TransactionReceipt::L1Handler(tx.parse_model(transaction_hash)?),
            Type::Declare(tx) => TransactionReceipt::Declare(tx.parse_model(transaction_hash)?),
            Type::DeprecatedDeploy(tx) => TransactionReceipt::Deploy(tx.parse_model(transaction_hash)?),
            Type::DeployAccount(tx) => TransactionReceipt::DeployAccount(tx.parse_model(transaction_hash)?),
        })
    }
}

impl model::receipt::Invoke {
    #[model_describe(model::receipt::Invoke)]
    pub fn parse_model(self, transaction_hash: Felt) -> Result<InvokeTransactionReceipt, FromModelError> {
        let common = model_field!(self => common);
        Ok(InvokeTransactionReceipt {
            transaction_hash,
            actual_fee: FeePayment {
                unit: common.price_unit().into(),
                amount: model_field!(common => actual_fee).into(),
            },
            messages_sent: common.messages_sent.into_iter().map(TryInto::try_into).collect::<Result<_, _>>()?,
            events: vec![],
            execution_resources: model_field!(common => execution_resources).try_into()?,
            execution_result: execution_result(common.revert_reason),
        })
    }
}

impl model::receipt::L1Handler {
    #[model_describe(model::receipt::L1Handler)]
    pub fn parse_model(self, transaction_hash: Felt) -> Result<L1HandlerTransactionReceipt, FromModelError> {
        let common = model_field!(self => common);
        Ok(L1HandlerTransactionReceipt {
            transaction_hash,
            actual_fee: FeePayment {
                unit: common.price_unit().into(),
                amount: model_field!(common => actual_fee).into(),
            },
            messages_sent: common.messages_sent.into_iter().map(TryInto::try_into).collect::<Result<_, _>>()?,
            events: vec![],
            execution_resources: model_field!(common => execution_resources).try_into()?,
            execution_result: execution_result(common.revert_reason),
            message_hash: model_field!(self => msg_hash).into(),
        })
    }
}

impl model::receipt::Declare {
    #[model_describe(model::receipt::Declare)]
    pub fn parse_model(self, transaction_hash: Felt) -> Result<DeclareTransactionReceipt, FromModelError> {
        let common = model_field!(self => common);
        Ok(DeclareTransactionReceipt {
            transaction_hash,
            actual_fee: FeePayment {
                unit: common.price_unit().into(),
                amount: model_field!(common => actual_fee).into(),
            },
            messages_sent: common.messages_sent.into_iter().map(TryInto::try_into).collect::<Result<_, _>>()?,
            events: vec![],
            execution_resources: model_field!(common => execution_resources).try_into()?,
            execution_result: execution_result(common.revert_reason),
        })
    }
}

impl model::receipt::Deploy {
    #[model_describe(model::receipt::Deploy)]
    pub fn parse_model(self, transaction_hash: Felt) -> Result<DeployTransactionReceipt, FromModelError> {
        let common = model_field!(self => common);
        Ok(DeployTransactionReceipt {
            transaction_hash,
            actual_fee: FeePayment {
                unit: common.price_unit().into(),
                amount: model_field!(common => actual_fee).into(),
            },
            messages_sent: common.messages_sent.into_iter().map(TryInto::try_into).collect::<Result<_, _>>()?,
            events: vec![],
            execution_resources: model_field!(common => execution_resources).try_into()?,
            execution_result: execution_result(common.revert_reason),
            contract_address: model_field!(self => contract_address).into(),
        })
    }
}

impl model::receipt::DeployAccount {
    #[model_describe(model::receipt::DeployAccount)]
    pub fn parse_model(self, transaction_hash: Felt) -> Result<DeployAccountTransactionReceipt, FromModelError> {
        let common = model_field!(self => common);
        Ok(DeployAccountTransactionReceipt {
            transaction_hash,
            actual_fee: FeePayment {
                unit: common.price_unit().into(),
                amount: model_field!(common => actual_fee).into(),
            },
            messages_sent: common.messages_sent.into_iter().map(TryInto::try_into).collect::<Result<_, _>>()?,
            events: vec![],
            execution_resources: model_field!(common => execution_resources).try_into()?,
            execution_result: execution_result(common.revert_reason),
            contract_address: model_field!(self => contract_address).into(),
        })
    }
}

impl TryFrom<model::MessageToL1> for MsgToL1 {
    type Error = FromModelError;

    #[model_describe(model::MessageToL1)]
    fn try_from(value: model::MessageToL1) -> Result<Self, Self::Error> {
        Ok(Self {
            from_address: model_field!(value => from_address).into(),
            to_address: model_field!(value => to_address).into(),
            payload: value.payload.collect_into(),
        })
    }
}

impl TryFrom<model::receipt::ExecutionResources> for ExecutionResources {
    type Error = FromModelError;

    #[model_describe(model::receipt::ExecutionResources)]
    fn try_from(value: model::receipt::ExecutionResources) -> Result<Self, Self::Error> {
        let builtins = model_field!(value => builtins);
        Ok(Self {
            steps: value.steps.into(),
            memory_holes: value.memory_holes.into(),
            range_check_builtin_applications: builtins.range_check.into(),
            pedersen_builtin_applications: builtins.pedersen.into(),
            poseidon_builtin_applications: builtins.poseidon.into(),
            ec_op_builtin_applications: builtins.ec_op.into(),
            ecdsa_builtin_applications: builtins.ecdsa.into(),
            bitwise_builtin_applications: builtins.bitwise.into(),
            keccak_builtin_applications: builtins.keccak.into(),
            // TODO: missing builtins (blockifier update needed)
            // TODO: what's that again? why is the naming convention different and why don't we have the field for it
            // segment_arena_builtin: builtins.,
            // segment_arena_builtin: builtins.segment_arena,
            segment_arena_builtin: 0,
            data_availability: L1Gas {
                l1_gas: model_field!(value => l1_gas).0.try_into_field("ExecutionResources::l1_gas")?,
                l1_data_gas: model_field!(value => l1_data_gas).0.try_into_field("ExecutionResources::l1_data_gas")?,
            },
            // TODO: wrong, update blockifier
            total_gas_consumed: L1Gas::default(),
            // l1_gas: ..
            // l1_data_gas: ..
            // total_l1_gas: ..
        })
    }
}

impl From<TransactionWithHash> for model::TransactionInBlock {
    fn from(value: TransactionWithHash) -> Self {
        Self { transaction_hash: Some(value.hash.into()), txn: Some(value.transaction.into()) }
    }
}

impl From<Transaction> for model::transaction_in_block::Txn {
    fn from(value: Transaction) -> Self {
        match value {
            Transaction::Invoke(tx) => match tx {
                InvokeTransaction::V0(tx) => Self::InvokeV0(tx.into()),
                InvokeTransaction::V1(tx) => Self::InvokeV1(tx.into()),
                InvokeTransaction::V3(tx) => Self::InvokeV3(tx.into()),
            },
            Transaction::L1Handler(tx) => Self::L1Handler(tx.into()),
            Transaction::Declare(tx) => match tx {
                DeclareTransaction::V0(tx) => Self::DeclareV0(tx.into()),
                DeclareTransaction::V1(tx) => Self::DeclareV1(tx.into()),
                DeclareTransaction::V2(tx) => Self::DeclareV2(tx.into()),
                DeclareTransaction::V3(tx) => Self::DeclareV3(tx.into()),
            },
            Transaction::Deploy(tx) => Self::Deploy(tx.into()),
            Transaction::DeployAccount(tx) => match tx {
                DeployAccountTransaction::V1(tx) => Self::DeployAccountV1(tx.into()),
                DeployAccountTransaction::V3(tx) => Self::DeployAccountV3(tx.into()),
            },
        }
    }
}

impl From<InvokeTransactionV0> for model::transaction_in_block::InvokeV0 {
    fn from(value: InvokeTransactionV0) -> Self {
        Self {
            max_fee: Some(value.max_fee.into()),
            signature: Some(model::AccountSignature { parts: value.signature.collect_into() }),
            address: Some(value.contract_address.into()),
            entry_point_selector: Some(value.entry_point_selector.into()),
            calldata: value.calldata.collect_into(),
        }
    }
}

impl From<InvokeTransactionV1> for model::transaction_in_block::InvokeV1 {
    fn from(value: InvokeTransactionV1) -> Self {
        Self {
            sender: Some(value.sender_address.into()),
            max_fee: Some(value.max_fee.into()),
            signature: Some(model::AccountSignature { parts: value.signature.collect_into() }),
            calldata: value.calldata.collect_into(),
            nonce: Some(value.nonce.into()),
        }
    }
}

impl From<InvokeTransactionV3> for model::InvokeV3 {
    fn from(value: InvokeTransactionV3) -> Self {
        Self {
            sender: Some(value.sender_address.into()),
            signature: Some(model::AccountSignature { parts: value.signature.collect_into() }),
            calldata: value.calldata.collect_into(),
            resource_bounds: Some(value.resource_bounds.into()),
            tip: value.tip,
            paymaster_data: value.paymaster_data.collect_into(),
            account_deployment_data: value.account_deployment_data.collect_into(),
            nonce_data_availability_mode: model::VolitionDomain::from(value.nonce_data_availability_mode).into(),
            fee_data_availability_mode: model::VolitionDomain::from(value.fee_data_availability_mode).into(),
            nonce: Some(value.nonce.into()),
        }
    }
}

impl From<L1HandlerTransaction> for model::L1HandlerV0 {
    fn from(value: L1HandlerTransaction) -> Self {
        Self {
            nonce: Some(Felt::from(value.nonce).into()),
            address: Some(value.contract_address.into()),
            entry_point_selector: Some(value.entry_point_selector.into()),
            calldata: value.calldata.collect_into(),
        }
    }
}

impl From<DeclareTransactionV0> for model::transaction_in_block::DeclareV0WithoutClass {
    fn from(value: DeclareTransactionV0) -> Self {
        Self {
            sender: Some(value.sender_address.into()),
            max_fee: Some(value.max_fee.into()),
            signature: Some(model::AccountSignature { parts: value.signature.collect_into() }),
            class_hash: Some(value.class_hash.into()),
        }
    }
}

impl From<DeclareTransactionV1> for model::transaction_in_block::DeclareV1WithoutClass {
    fn from(value: DeclareTransactionV1) -> Self {
        Self {
            sender: Some(value.sender_address.into()),
            max_fee: Some(value.max_fee.into()),
            signature: Some(model::AccountSignature { parts: value.signature.collect_into() }),
            class_hash: Some(value.class_hash.into()),
            nonce: Some(value.nonce.into()),
        }
    }
}

impl From<DeclareTransactionV2> for model::transaction_in_block::DeclareV2WithoutClass {
    fn from(value: DeclareTransactionV2) -> Self {
        Self {
            sender: Some(value.sender_address.into()),
            max_fee: Some(value.max_fee.into()),
            signature: Some(model::AccountSignature { parts: value.signature.collect_into() }),
            class_hash: Some(value.class_hash.into()),
            nonce: Some(value.nonce.into()),
            compiled_class_hash: Some(value.compiled_class_hash.into()),
        }
    }
}

impl From<DeclareTransactionV3> for model::transaction_in_block::DeclareV3WithoutClass {
    fn from(value: DeclareTransactionV3) -> Self {
        Self {
            class_hash: Some(value.class_hash.into()),
            common: Some(model::DeclareV3Common {
                sender: Some(value.sender_address.into()),
                signature: Some(model::AccountSignature { parts: value.signature.collect_into() }),
                nonce: Some(value.nonce.into()),
                compiled_class_hash: Some(value.compiled_class_hash.into()),
                resource_bounds: Some(value.resource_bounds.into()),
                tip: value.tip,
                paymaster_data: value.paymaster_data.collect_into(),
                account_deployment_data: value.account_deployment_data.collect_into(),
                nonce_data_availability_mode: model::VolitionDomain::from(value.nonce_data_availability_mode).into(),
                fee_data_availability_mode: model::VolitionDomain::from(value.fee_data_availability_mode).into(),
            }),
        }
    }
}

impl From<DeployTransaction> for model::transaction_in_block::Deploy {
    fn from(value: DeployTransaction) -> Self {
        Self {
            class_hash: Some(value.class_hash.into()),
            address_salt: Some(value.contract_address_salt.into()),
            calldata: value.constructor_calldata.collect_into(),
            // TODO(dto-faillible-conversion)
            version: value.version.try_into().expect("DeployTransaction version is not an u32"),
        }
    }
}

impl From<DeployAccountTransactionV1> for model::transaction_in_block::DeployAccountV1 {
    fn from(value: DeployAccountTransactionV1) -> Self {
        Self {
            max_fee: Some(value.max_fee.into()),
            signature: Some(model::AccountSignature { parts: value.signature.collect_into() }),
            class_hash: Some(value.class_hash.into()),
            nonce: Some(value.nonce.into()),
            address_salt: Some(value.contract_address_salt.into()),
            calldata: value.constructor_calldata.collect_into(),
        }
    }
}

impl From<DeployAccountTransactionV3> for model::DeployAccountV3 {
    fn from(value: DeployAccountTransactionV3) -> Self {
        Self {
            signature: Some(model::AccountSignature { parts: value.signature.collect_into() }),
            class_hash: Some(value.class_hash.into()),
            nonce: Some(value.nonce.into()),
            address_salt: Some(value.contract_address_salt.into()),
            calldata: value.constructor_calldata.collect_into(),
            resource_bounds: Some(value.resource_bounds.into()),
            tip: value.tip,
            paymaster_data: value.paymaster_data.collect_into(),
            nonce_data_availability_mode: model::VolitionDomain::from(value.nonce_data_availability_mode).into(),
            fee_data_availability_mode: model::VolitionDomain::from(value.fee_data_availability_mode).into(),
        }
    }
}

impl From<ResourceBoundsMapping> for model::ResourceBounds {
    fn from(value: ResourceBoundsMapping) -> Self {
        Self {
            l1_gas: Some(value.l1_gas.into()),
            l2_gas: Some(value.l2_gas.into()),
            l1_data_gas: None, // TODO: Update blockifier
        }
    }
}

impl From<ResourceBounds> for model::ResourceLimits {
    fn from(value: ResourceBounds) -> Self {
        Self {
            max_amount: Some(Felt::from(value.max_amount).into()),
            max_price_per_unit: Some(Felt::from(value.max_price_per_unit).into()),
        }
    }
}

impl From<DataAvailabilityMode> for model::VolitionDomain {
    fn from(value: DataAvailabilityMode) -> Self {
        match value {
            DataAvailabilityMode::L1 => model::VolitionDomain::L1,
            DataAvailabilityMode::L2 => model::VolitionDomain::L2,
        }
    }
}

impl From<TransactionReceipt> for model::Receipt {
    fn from(value: TransactionReceipt) -> Self {
        use model::receipt::Type;
        Self {
            r#type: Some(match value {
                TransactionReceipt::Invoke(receipt) => Type::Invoke(receipt.into()),
                TransactionReceipt::L1Handler(receipt) => Type::L1Handler(receipt.into()),
                TransactionReceipt::Declare(receipt) => Type::Declare(receipt.into()),
                TransactionReceipt::Deploy(receipt) => Type::DeprecatedDeploy(receipt.into()),
                TransactionReceipt::DeployAccount(receipt) => Type::DeployAccount(receipt.into()),
            }),
        }
    }
}

impl From<InvokeTransactionReceipt> for model::receipt::Invoke {
    fn from(value: InvokeTransactionReceipt) -> Self {
        Self {
            common: Some(model::receipt::Common {
                actual_fee: Some(value.actual_fee.amount.into()),
                price_unit: model::PriceUnit::from(value.actual_fee.unit).into(),
                messages_sent: value.messages_sent.collect_into(),
                execution_resources: Some(value.execution_resources.into()),
                revert_reason: value.execution_result.revert_reason().map(String::from),
            }),
        }
    }
}

impl From<L1HandlerTransactionReceipt> for model::receipt::L1Handler {
    fn from(value: L1HandlerTransactionReceipt) -> Self {
        Self {
            common: Some(model::receipt::Common {
                actual_fee: Some(value.actual_fee.amount.into()),
                price_unit: model::PriceUnit::from(value.actual_fee.unit).into(),
                messages_sent: value.messages_sent.collect_into(),
                execution_resources: Some(value.execution_resources.into()),
                revert_reason: value.execution_result.revert_reason().map(String::from),
            }),
            msg_hash: Some(value.message_hash.into()),
        }
    }
}

impl From<DeclareTransactionReceipt> for model::receipt::Declare {
    fn from(value: DeclareTransactionReceipt) -> Self {
        Self {
            common: Some(model::receipt::Common {
                actual_fee: Some(value.actual_fee.amount.into()),
                price_unit: model::PriceUnit::from(value.actual_fee.unit).into(),
                messages_sent: value.messages_sent.collect_into(),
                execution_resources: Some(value.execution_resources.into()),
                revert_reason: value.execution_result.revert_reason().map(String::from),
            }),
        }
    }
}

impl From<DeployTransactionReceipt> for model::receipt::Deploy {
    fn from(value: DeployTransactionReceipt) -> Self {
        Self {
            common: Some(model::receipt::Common {
                actual_fee: Some(value.actual_fee.amount.into()),
                price_unit: model::PriceUnit::from(value.actual_fee.unit).into(),
                messages_sent: value.messages_sent.collect_into(),
                execution_resources: Some(value.execution_resources.into()),
                revert_reason: value.execution_result.revert_reason().map(String::from),
            }),
            contract_address: Some(value.contract_address.into()),
        }
    }
}

impl From<DeployAccountTransactionReceipt> for model::receipt::DeployAccount {
    fn from(value: DeployAccountTransactionReceipt) -> Self {
        Self {
            common: Some(model::receipt::Common {
                actual_fee: Some(value.actual_fee.amount.into()),
                price_unit: model::PriceUnit::from(value.actual_fee.unit).into(),
                messages_sent: value.messages_sent.collect_into(),
                execution_resources: Some(value.execution_resources.into()),
                revert_reason: value.execution_result.revert_reason().map(String::from),
            }),
            contract_address: Some(value.contract_address.into()),
        }
    }
}

impl From<MsgToL1> for model::MessageToL1 {
    fn from(value: MsgToL1) -> Self {
        Self {
            from_address: Some(value.from_address.into()),
            payload: value.payload.collect_into(),
            to_address: Some(value.to_address.into()),
        }
    }
}

impl From<ExecutionResources> for model::receipt::ExecutionResources {
    fn from(value: ExecutionResources) -> Self {
        Self {
            // TODO(dto-faillible-conversion)
            builtins: Some(BuiltinCounter {
                bitwise: value.bitwise_builtin_applications.try_into().expect("bitwise_builtin > u32::MAX"),
                ecdsa: value.ecdsa_builtin_applications.try_into().expect("ecdsa_builtin > u32::MAX"),
                ec_op: value.ec_op_builtin_applications.try_into().expect("ec_op_builtin > u32::MAX"),
                pedersen: value.pedersen_builtin_applications.try_into().expect("pedersen_builtin > u32::MAX"),
                range_check: value.range_check_builtin_applications.try_into().expect("range_check_builtin > u32::MAX"),
                poseidon: value.poseidon_builtin_applications.try_into().expect("poseidon_builtin > u32::MAX"),
                keccak: value.keccak_builtin_applications.try_into().expect("keccak_builtin > u32::MAX"),
                // TODO: missing builtins
                ..Default::default()
            }),
            // TODO(dto-faillible-conversion)
            steps: value.steps.try_into().expect("steps > u32::MAX"),
            // TODO(dto-faillible-conversion)
            memory_holes: value.memory_holes.try_into().expect("memory_holes > u32::MAX"),
            l1_gas: Some(Felt::from(value.total_gas_consumed.l1_gas).into()),
            l1_data_gas: Some(Felt::from(value.total_gas_consumed.l1_data_gas).into()),
            total_l1_gas: Some(Felt::from(value.total_gas_consumed.l1_gas).into()),
            l2_gas: None, // TODO: update blockifier
        }
    }
}

impl From<PriceUnit> for model::PriceUnit {
    fn from(value: PriceUnit) -> Self {
        match value {
            PriceUnit::Wei => Self::Wei,
            PriceUnit::Fri => Self::Fri,
        }
    }
}
impl From<model::PriceUnit> for PriceUnit {
    fn from(value: model::PriceUnit) -> Self {
        match value {
            model::PriceUnit::Wei => Self::Wei,
            model::PriceUnit::Fri => Self::Fri,
        }
    }
}
