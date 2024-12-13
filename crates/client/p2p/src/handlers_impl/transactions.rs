use crate::{
    handlers_impl::{
        block_stream_config,
        error::{OptionExt, ResultExt},
    },
    model::{self, receipt::execution_resources::BuiltinCounter},
    sync_handlers::{self, ReqContext},
    MadaraP2pContext,
};
use futures::{channel::mpsc::Sender, SinkExt, StreamExt};
use mc_db::db_block_id::DbBlockId;
use mp_convert::felt_to_u32;
use mp_receipt::{
    DeclareTransactionReceipt, DeployAccountTransactionReceipt, DeployTransactionReceipt, ExecutionResources,
    InvokeTransactionReceipt, L1HandlerTransactionReceipt, MsgToL1, PriceUnit, TransactionReceipt,
};
use mp_transactions::{
    DataAvailabilityMode, DeclareTransaction, DeclareTransactionV0, DeclareTransactionV1, DeclareTransactionV2,
    DeclareTransactionV3, DeployAccountTransaction, DeployAccountTransactionV1, DeployAccountTransactionV3,
    DeployTransaction, InvokeTransaction, InvokeTransactionV0, InvokeTransactionV1, InvokeTransactionV3,
    L1HandlerTransaction, ResourceBounds, ResourceBoundsMapping, Transaction, TransactionWithHash,
};
use starknet_core::types::Felt;
use tokio::pin;

impl From<TransactionWithHash> for model::Transaction {
    fn from(value: TransactionWithHash) -> Self {
        Self { transaction_hash: Some(value.hash.into()), txn: Some(value.transaction.into()) }
    }
}

impl From<Transaction> for model::transaction::Txn {
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

impl From<InvokeTransactionV0> for model::transaction::InvokeV0 {
    fn from(value: InvokeTransactionV0) -> Self {
        Self {
            max_fee: Some(value.max_fee.into()),
            signature: Some(model::AccountSignature { parts: value.signature.into_iter().map(Into::into).collect() }),
            address: Some(value.contract_address.into()),
            entry_point_selector: Some(value.entry_point_selector.into()),
            calldata: value.calldata.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<InvokeTransactionV1> for model::transaction::InvokeV1 {
    fn from(value: InvokeTransactionV1) -> Self {
        Self {
            sender: Some(value.sender_address.into()),
            max_fee: Some(value.max_fee.into()),
            signature: Some(model::AccountSignature { parts: value.signature.into_iter().map(Into::into).collect() }),
            calldata: value.calldata.into_iter().map(Into::into).collect(),
            nonce: Some(value.nonce.into()),
        }
    }
}

impl From<InvokeTransactionV3> for model::transaction::InvokeV3 {
    fn from(value: InvokeTransactionV3) -> Self {
        Self {
            sender: Some(value.sender_address.into()),
            signature: Some(model::AccountSignature { parts: value.signature.into_iter().map(Into::into).collect() }),
            calldata: value.calldata.into_iter().map(Into::into).collect(),
            resource_bounds: Some(value.resource_bounds.into()),
            tip: value.tip,
            paymaster_data: value.paymaster_data.into_iter().map(Into::into).collect(),
            account_deployment_data: value.account_deployment_data.into_iter().map(Into::into).collect(),
            nonce_data_availability_mode: model::VolitionDomain::from(value.nonce_data_availability_mode).into(),
            fee_data_availability_mode: model::VolitionDomain::from(value.fee_data_availability_mode).into(),
            nonce: Some(value.nonce.into()),
        }
    }
}

impl From<L1HandlerTransaction> for model::transaction::L1HandlerV0 {
    fn from(value: L1HandlerTransaction) -> Self {
        Self {
            nonce: Some(Felt::from(value.nonce).into()),
            address: Some(value.contract_address.into()),
            entry_point_selector: Some(value.entry_point_selector.into()),
            calldata: value.calldata.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<DeclareTransactionV0> for model::transaction::DeclareV0 {
    fn from(value: DeclareTransactionV0) -> Self {
        Self {
            sender: Some(value.sender_address.into()),
            max_fee: Some(value.max_fee.into()),
            signature: Some(model::AccountSignature { parts: value.signature.into_iter().map(Into::into).collect() }),
            class_hash: Some(value.class_hash.into()),
        }
    }
}

impl From<DeclareTransactionV1> for model::transaction::DeclareV1 {
    fn from(value: DeclareTransactionV1) -> Self {
        Self {
            sender: Some(value.sender_address.into()),
            max_fee: Some(value.max_fee.into()),
            signature: Some(model::AccountSignature { parts: value.signature.into_iter().map(Into::into).collect() }),
            class_hash: Some(value.class_hash.into()),
            nonce: Some(value.nonce.into()),
        }
    }
}

impl From<DeclareTransactionV2> for model::transaction::DeclareV2 {
    fn from(value: DeclareTransactionV2) -> Self {
        Self {
            sender: Some(value.sender_address.into()),
            max_fee: Some(value.max_fee.into()),
            signature: Some(model::AccountSignature { parts: value.signature.into_iter().map(Into::into).collect() }),
            class_hash: Some(value.class_hash.into()),
            nonce: Some(value.nonce.into()),
            compiled_class_hash: Some(value.compiled_class_hash.into()),
        }
    }
}

impl From<DeclareTransactionV3> for model::transaction::DeclareV3 {
    fn from(value: DeclareTransactionV3) -> Self {
        Self {
            sender: Some(value.sender_address.into()),
            signature: Some(model::AccountSignature { parts: value.signature.into_iter().map(Into::into).collect() }),
            class_hash: Some(value.class_hash.into()),
            nonce: Some(value.nonce.into()),
            compiled_class_hash: Some(value.compiled_class_hash.into()),
            resource_bounds: Some(value.resource_bounds.into()),
            tip: value.tip,
            paymaster_data: value.paymaster_data.into_iter().map(Into::into).collect(),
            account_deployment_data: value.account_deployment_data.into_iter().map(Into::into).collect(),
            nonce_data_availability_mode: model::VolitionDomain::from(value.nonce_data_availability_mode).into(),
            fee_data_availability_mode: model::VolitionDomain::from(value.fee_data_availability_mode).into(),
        }
    }
}

impl From<DeployTransaction> for model::transaction::Deploy {
    fn from(value: DeployTransaction) -> Self {
        Self {
            class_hash: Some(value.class_hash.into()),
            address_salt: Some(value.contract_address_salt.into()),
            calldata: value.constructor_calldata.into_iter().map(Into::into).collect(),
            // TODO(dto-faillible-conversion)
            version: felt_to_u32(&value.version).expect("DeployTransaction version is not an u32"),
        }
    }
}

impl From<DeployAccountTransactionV1> for model::transaction::DeployAccountV1 {
    fn from(value: DeployAccountTransactionV1) -> Self {
        Self {
            max_fee: Some(value.max_fee.into()),
            signature: Some(model::AccountSignature { parts: value.signature.into_iter().map(Into::into).collect() }),
            class_hash: Some(value.class_hash.into()),
            nonce: Some(value.nonce.into()),
            address_salt: Some(value.contract_address_salt.into()),
            calldata: value.constructor_calldata.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<DeployAccountTransactionV3> for model::transaction::DeployAccountV3 {
    fn from(value: DeployAccountTransactionV3) -> Self {
        Self {
            signature: Some(model::AccountSignature { parts: value.signature.into_iter().map(Into::into).collect() }),
            class_hash: Some(value.class_hash.into()),
            nonce: Some(value.nonce.into()),
            address_salt: Some(value.contract_address_salt.into()),
            calldata: value.constructor_calldata.into_iter().map(Into::into).collect(),
            resource_bounds: Some(value.resource_bounds.into()),
            tip: value.tip,
            paymaster_data: value.paymaster_data.into_iter().map(Into::into).collect(),
            nonce_data_availability_mode: model::VolitionDomain::from(value.nonce_data_availability_mode).into(),
            fee_data_availability_mode: model::VolitionDomain::from(value.fee_data_availability_mode).into(),
        }
    }
}

impl From<ResourceBoundsMapping> for model::ResourceBounds {
    fn from(value: ResourceBoundsMapping) -> Self {
        Self { l1_gas: Some(value.l1_gas.into()), l2_gas: Some(value.l2_gas.into()) }
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
                messages_sent: value.messages_sent.into_iter().map(Into::into).collect(),
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
                messages_sent: value.messages_sent.into_iter().map(Into::into).collect(),
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
                messages_sent: value.messages_sent.into_iter().map(Into::into).collect(),
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
                messages_sent: value.messages_sent.into_iter().map(Into::into).collect(),
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
                messages_sent: value.messages_sent.into_iter().map(Into::into).collect(),
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
            payload: value.payload.into_iter().map(Into::into).collect(),
            to_address: Some(value.to_address.into()),
        }
    }
}

impl From<ExecutionResources> for model::receipt::ExecutionResources {
    fn from(value: ExecutionResources) -> Self {
        Self {
            // TODO(dto-faillible-conversion)
            builtins: Some(BuiltinCounter {
                bitwise: value
                    .bitwise_builtin_applications
                    .unwrap_or_default()
                    .try_into()
                    .expect("bitwise_builtin > u32::MAX"),
                ecdsa: value
                    .ecdsa_builtin_applications
                    .unwrap_or_default()
                    .try_into()
                    .expect("ecdsa_builtin > u32::MAX"),
                ec_op: value
                    .ec_op_builtin_applications
                    .unwrap_or_default()
                    .try_into()
                    .expect("ec_op_builtin > u32::MAX"),
                pedersen: value
                    .pedersen_builtin_applications
                    .unwrap_or_default()
                    .try_into()
                    .expect("pedersen_builtin > u32::MAX"),
                range_check: value
                    .range_check_builtin_applications
                    .unwrap_or_default()
                    .try_into()
                    .expect("range_check_builtin > u32::MAX"),
                poseidon: value
                    .poseidon_builtin_applications
                    .unwrap_or_default()
                    .try_into()
                    .expect("poseidon_builtin > u32::MAX"),
                keccak: value
                    .keccak_builtin_applications
                    .unwrap_or_default()
                    .try_into()
                    .expect("keccak_builtin > u32::MAX"),
                // TODO: missing builtins
                // output: value.output_builtin_applications.unwrap_or_default().try_into().expect("output_builtin > u32::MAX"),
                // add_mod: value.add_mod_builtin_applications.unwrap_or_default().try_into().expect("add_mod_builtin > u32::MAX"),
                // mul_mod: value.mul_mod_builtin_applications.unwrap_or_default().try_into().expect("mul_mod_builtin > u32::MAX"),
                // range_check96: value
                //     .range_check96_builtin_applications
                //     .unwrap_or_default().try_into().expect("range_check96_builtin > u32::MAX"),
                ..Default::default()
            }),
            // TODO(dto-faillible-conversion)
            steps: value.steps.try_into().expect("steps > u32::MAX"),
            // TODO(dto-faillible-conversion)
            memory_holes: value.memory_holes.unwrap_or(0).try_into().expect("memory_holes > u32::MAX"),
            l1_gas: Some(Felt::from(value.total_gas_consumed.l1_gas).into()),
            l1_data_gas: Some(Felt::from(value.total_gas_consumed.l1_data_gas).into()),
            total_l1_gas: Some(Felt::from(value.total_gas_consumed.l1_gas).into()),
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

pub async fn transactions_sync(
    ctx: ReqContext<MadaraP2pContext>,
    req: model::TransactionsRequest,
    mut out: Sender<model::TransactionsResponse>,
) -> Result<(), sync_handlers::Error> {
    let stream = ctx
        .app_ctx
        .backend
        .block_info_stream(block_stream_config(&ctx.app_ctx.backend, req.iteration.unwrap_or_default())?);
    pin!(stream);

    tracing::debug!("transactions sync!");

    while let Some(res) = stream.next().await {
        let header = res.or_internal_server_error("Error while reading from block stream")?;

        let block_inner = ctx
            .app_ctx
            .backend
            .get_block_inner(&DbBlockId::Number(header.header.block_number))
            .or_internal_server_error("Getting block state diff")?
            .ok_or_internal_server_error("No body for block")?;

        for ((transaction, &hash), receipt) in
            block_inner.transactions.iter().zip(header.tx_hashes.iter()).zip(&block_inner.receipts)
        {
            let el = model::TransactionWithReceipt {
                transaction: Some(TransactionWithHash { transaction: transaction.clone(), hash }.into()),
                receipt: Some(receipt.clone().into()),
            };

            out.send(model::TransactionsResponse {
                transaction_message: Some(model::transactions_response::TransactionMessage::TransactionWithReceipt(el)),
            })
            .await?
        }
    }

    // Add the Fin message
    out.send(model::TransactionsResponse {
        transaction_message: Some(model::transactions_response::TransactionMessage::Fin(model::Fin {})),
    })
    .await?;

    Ok(())
}
