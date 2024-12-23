use std::sync::Arc;

use bytes::Buf;
use http_body_util::BodyExt;
use hyper::{body::Incoming, Request, Response};
use mc_db::MadaraBackend;
use mc_rpc::{
    providers::AddTransactionProvider,
    versions::user::v0_7_1::methods::trace::trace_block_transactions::trace_block_transactions as v0_7_1_trace_block_transactions,
    Starknet,
};
use mp_block::{BlockId, BlockTag, MadaraBlock, MadaraMaybePendingBlockInfo, MadaraPendingBlock};
use mp_class::{ClassInfo, ContractClass};
use mp_gateway::error::StarknetError;
use mp_gateway::user_transaction::{
    UserDeclareTransaction, UserDeployAccountTransaction, UserInvokeFunctionTransaction, UserTransaction,
};
use mp_gateway::{
    block::{BlockStatus, ProviderBlock, ProviderBlockPending, ProviderBlockSignature},
    state_update::{ProviderStateUpdate, ProviderStateUpdatePending},
};
use mp_utils::service::ServiceContext;
use serde::Serialize;
use serde_json::json;
use starknet_types_core::felt::Felt;
use starknet_types_rpc::{BroadcastedDeclareTxn, TraceBlockTransactionsResult};

use super::{
    error::{GatewayError, OptionExt, ResultExt},
    helpers::{
        block_id_from_params, create_json_response, create_response_with_json_body, create_string_response,
        get_params_from_request, include_block_params,
    },
};

pub async fn handle_get_block(
    req: Request<Incoming>,
    backend: Arc<MadaraBackend>,
) -> Result<Response<String>, GatewayError> {
    let params = get_params_from_request(&req);
    let block_id = block_id_from_params(&params).or_internal_server_error("Retrieving block id")?;

    if params.get("headerOnly").map(|s| s.as_ref()) == Some("true") {
        if matches!(block_id, BlockId::Tag(BlockTag::Pending)) {
            return Err(GatewayError::StarknetError(StarknetError::no_block_header_for_pending_block()));
        }

        let block_info = backend
            .get_block_info(&block_id)
            .or_internal_server_error(format!("Retrieving block {block_id:?}"))?
            .ok_or(StarknetError::block_not_found())?;

        match block_info {
            MadaraMaybePendingBlockInfo::Pending(_) => Err(GatewayError::InternalServerError(format!(
                "Retrieved pending block info from db for non-pending block {block_id:?}"
            ))),
            MadaraMaybePendingBlockInfo::NotPending(block_info) => {
                let body = json!({
                    "block_hash": block_info.block_hash,
                    "block_number": block_info.header.block_number
                });
                Ok(create_json_response(hyper::StatusCode::OK, &body))
            }
        }
    } else {
        let block = backend
            .get_block(&block_id)
            .or_internal_server_error(format!("Retrieving block {block_id:?}"))?
            .ok_or(StarknetError::block_not_found())?;

        if let Ok(block) = MadaraBlock::try_from(block.clone()) {
            let last_l1_confirmed_block =
                backend.get_l1_last_confirmed_block().or_internal_server_error("Retrieving last l1 confirmed block")?;

            let status = if Some(block.info.header.block_number) <= last_l1_confirmed_block {
                BlockStatus::AcceptedOnL1
            } else {
                BlockStatus::AcceptedOnL2
            };

            let block_provider = ProviderBlock::new(block, status);
            Ok(create_json_response(hyper::StatusCode::OK, &block_provider))
        } else {
            let block =
                MadaraPendingBlock::try_from(block).map_err(|e| GatewayError::InternalServerError(e.to_string()))?;
            let block_provider = ProviderBlockPending::new(block);
            Ok(create_json_response(hyper::StatusCode::OK, &block_provider))
        }
    }
}

pub async fn handle_get_signature(
    req: Request<Incoming>,
    backend: Arc<MadaraBackend>,
) -> Result<Response<String>, GatewayError> {
    let params = get_params_from_request(&req);
    let block_id = block_id_from_params(&params).or_internal_server_error("Retrieving block id")?;

    if matches!(block_id, BlockId::Tag(BlockTag::Pending)) {
        return Err(GatewayError::StarknetError(StarknetError::no_signature_for_pending_block()));
    }

    let block_info = backend
        .get_block_info(&block_id)
        .or_internal_server_error(format!("Retrieving block info for block {block_id:?}"))?
        .ok_or(StarknetError::block_not_found())?;

    match block_info {
        MadaraMaybePendingBlockInfo::Pending(_) => Err(GatewayError::InternalServerError(format!(
            "Retrieved pending block info from db for non-pending block {block_id:?}"
        ))),
        MadaraMaybePendingBlockInfo::NotPending(block_info) => {
            let private_key = &backend.chain_config().private_key;
            let signature = private_key
                .sign(&block_info.block_hash)
                .map_err(|e| GatewayError::InternalServerError(format!("Failed to sign block hash: {e}")))?;
            let signature =
                ProviderBlockSignature { block_hash: block_info.block_hash, signature: vec![signature.r, signature.s] };
            Ok(create_json_response(hyper::StatusCode::OK, &signature))
        }
    }
}

pub async fn handle_get_state_update(
    req: Request<Incoming>,
    backend: Arc<MadaraBackend>,
) -> Result<Response<String>, GatewayError> {
    let params = get_params_from_request(&req);
    let block_id = block_id_from_params(&params).or_internal_server_error("Retrieving block id")?;

    let resolved_block_id = backend
        .resolve_block_id(&block_id)
        .or_internal_server_error("Resolving block id from database")?
        .ok_or(StarknetError::block_not_found())?;

    let state_diff = backend
        .get_block_state_diff(&resolved_block_id)
        .or_internal_server_error("Retrieving state diff")?
        .ok_or(StarknetError::block_not_found())?;

    let with_block = if include_block_params(&params) {
        let block = backend
            .get_block(&block_id)
            .or_internal_server_error("Retrieving block {block_id}")?
            .ok_or(StarknetError::block_not_found())?;
        Some(block)
    } else {
        None
    };

    match resolved_block_id.is_pending() {
        true => {
            let old_root = backend
                .get_block_info(&BlockId::Tag(BlockTag::Latest))
                .or_internal_server_error("Retrieving old state root on latest block")?
                .map(|block| {
                    block
                        .as_nonpending()
                        .map(|block| block.header.global_state_root)
                        .ok_or_internal_server_error("Converting block to non-pending")
                })
                .unwrap_or(Ok(Felt::ZERO))?;

            let state_update = ProviderStateUpdatePending { old_root, state_diff: state_diff.into() };

            let json_response = if let Some(block) = with_block {
                let block = MadaraPendingBlock::try_from(block.clone())
                    .or_internal_server_error("Attempting to convert pending block to non-pending")?;
                let block_provider = ProviderBlockPending::new(block);

                create_json_response(
                    hyper::StatusCode::OK,
                    &json!({"block": block_provider, "state_update": state_update}),
                )
            } else {
                create_json_response(hyper::StatusCode::OK, &state_update)
            };

            Ok(json_response)
        }
        false => {
            let resolved_block = backend
                .get_block_info(&resolved_block_id)
                .or_internal_server_error(format!("Retrieving block info at block {resolved_block_id}"))?
                .ok_or(StarknetError::block_not_found())?;
            let block_info = resolved_block
                .as_nonpending()
                .ok_or_internal_server_error("Converting potentially pending block to non-pending")?;

            let old_root = match block_info.header.block_number.checked_sub(1) {
                Some(val) => backend
                    .get_block_info(&BlockId::Number(val))
                    .or_internal_server_error("Retrieving old state root on latest block")?
                    .map(|block| {
                        block
                            .as_nonpending()
                            .map(|block| block.header.global_state_root)
                            .ok_or_internal_server_error("Converting block to non-pending")
                    })
                    .unwrap_or(Ok(Felt::ZERO))?,
                None => Felt::ZERO,
            };

            let state_update = ProviderStateUpdate {
                block_hash: block_info.block_hash,
                old_root,
                new_root: block_info.header.global_state_root,
                state_diff: state_diff.into(),
            };

            let json_response = if let Some(block) = with_block {
                let block = MadaraBlock::try_from(block.clone())
                    .or_internal_server_error("Attempting to convert pending block to non-pending")?;
                let block_provider = ProviderBlock::new(block, BlockStatus::AcceptedOnL2);

                create_json_response(
                    hyper::StatusCode::OK,
                    &json!({"block": block_provider, "state_update": state_update}),
                )
            } else {
                create_json_response(hyper::StatusCode::OK, &state_update)
            };

            Ok(json_response)
        }
    }
}

pub async fn handle_get_block_traces(
    req: Request<Incoming>,
    backend: Arc<MadaraBackend>,
    add_transaction_provider: Arc<dyn AddTransactionProvider>,
    ctx: ServiceContext,
) -> Result<Response<String>, GatewayError> {
    let params = get_params_from_request(&req);
    let block_id = block_id_from_params(&params).or_internal_server_error("Retrieving block id")?;

    #[derive(Serialize)]
    struct BlockTraces {
        traces: Vec<TraceBlockTransactionsResult<Felt>>,
    }

    let traces = v0_7_1_trace_block_transactions(
        &Starknet::new(backend, add_transaction_provider, Default::default(), ctx),
        block_id,
    )
    .await?;
    let block_traces = BlockTraces { traces };

    Ok(create_json_response(hyper::StatusCode::OK, &block_traces))
}

pub async fn handle_get_class_by_hash(
    req: Request<Incoming>,
    backend: Arc<MadaraBackend>,
) -> Result<Response<String>, GatewayError> {
    let params = get_params_from_request(&req);
    let block_id = block_id_from_params(&params).unwrap_or(BlockId::Tag(BlockTag::Latest));

    let class_hash = params.get("classHash").ok_or(StarknetError::missing_class_hash())?;
    let class_hash = Felt::from_hex(class_hash).map_err(StarknetError::invalid_class_hash)?;

    let class_info = backend
        .get_class_info(&block_id, &class_hash)
        .or_internal_server_error(format!("Retrieving class info from class hash {class_hash:x}"))?
        .ok_or(StarknetError::class_not_found(class_hash))?;

    let json_response = match class_info.contract_class() {
        ContractClass::Sierra(flattened_sierra_class) => {
            create_json_response(hyper::StatusCode::OK, flattened_sierra_class.as_ref())
        }
        ContractClass::Legacy(compressed_legacy_contract_class) => {
            let class = compressed_legacy_contract_class
                .as_ref()
                .serialize_to_json()
                .or_internal_server_error("Failed to serialize legacy class")?;
            create_response_with_json_body(hyper::StatusCode::OK, class)
        }
    };

    Ok(json_response)
}

pub async fn handle_get_compiled_class_by_class_hash(
    req: Request<Incoming>,
    backend: Arc<MadaraBackend>,
) -> Result<Response<String>, GatewayError> {
    let params = get_params_from_request(&req);
    let block_id = block_id_from_params(&params).unwrap_or(BlockId::Tag(BlockTag::Latest));

    let class_hash = params.get("classHash").ok_or(StarknetError::missing_class_hash())?;
    let class_hash = Felt::from_hex(class_hash).map_err(StarknetError::invalid_class_hash)?;

    let class_info = backend
        .get_class_info(&block_id, &class_hash)
        .or_internal_server_error(format!("Retrieving class info from class hash {class_hash:x}"))?
        .ok_or(StarknetError::class_not_found(class_hash))?;

    let compiled_class_hash = match class_info {
        ClassInfo::Sierra(class_info) => class_info.compiled_class_hash,
        ClassInfo::Legacy(_) => {
            return Err(GatewayError::StarknetError(StarknetError::sierra_class_not_found(class_hash)))
        }
    };

    let class_compiled = backend
        .get_sierra_compiled(&block_id, &compiled_class_hash)
        .or_internal_server_error(format!("Retrieving compiled Sierra class from class hash {class_hash:x}"))?
        .ok_or(StarknetError::class_not_found(class_hash))?;

    Ok(create_response_with_json_body(hyper::StatusCode::OK, class_compiled.0))
}

pub async fn handle_get_contract_addresses(backend: Arc<MadaraBackend>) -> Result<Response<String>, GatewayError> {
    let chain_config = &backend.chain_config();
    Ok(create_json_response(
        hyper::StatusCode::OK,
        &json!({
            "Starknet": chain_config.eth_core_contract_address,
            "GpsStatementVerifier": chain_config.eth_gps_statement_verifier
        }),
    ))
}

pub async fn handle_get_public_key(backend: Arc<MadaraBackend>) -> Result<Response<String>, GatewayError> {
    let public_key = &backend.chain_config().private_key.public;
    Ok(create_string_response(hyper::StatusCode::OK, format!("\"{:#x}\"", public_key)))
}

pub async fn handle_add_transaction(
    req: Request<Incoming>,
    add_transaction_provider: Arc<dyn AddTransactionProvider>,
) -> Result<Response<String>, GatewayError> {
    let whole_body = req.collect().await.or_internal_server_error("Failed to read request body")?.aggregate();

    let transaction = serde_json::from_reader::<_, UserTransaction>(whole_body.reader())
        .map_err(|e| GatewayError::StarknetError(StarknetError::malformed_request(e)))?;

    let response = match transaction {
        UserTransaction::Declare(tx) => declare_transaction(tx, add_transaction_provider).await,
        UserTransaction::DeployAccount(tx) => deploy_account_transaction(tx, add_transaction_provider).await,
        UserTransaction::InvokeFunction(tx) => invoke_transaction(tx, add_transaction_provider).await,
    };

    Ok(response)
}

async fn declare_transaction(
    tx: UserDeclareTransaction,
    add_transaction_provider: Arc<dyn AddTransactionProvider>,
) -> Response<String> {
    let tx: BroadcastedDeclareTxn<Felt> = tx.try_into().unwrap();

    match add_transaction_provider.add_declare_transaction(tx).await {
        Ok(result) => create_json_response(hyper::StatusCode::OK, &result),
        Err(e) => create_json_response(hyper::StatusCode::OK, &e),
    }
}

async fn deploy_account_transaction(
    tx: UserDeployAccountTransaction,
    add_transaction_provider: Arc<dyn AddTransactionProvider>,
) -> Response<String> {
    match add_transaction_provider.add_deploy_account_transaction(tx.into()).await {
        Ok(result) => create_json_response(hyper::StatusCode::OK, &result),
        Err(e) => create_json_response(hyper::StatusCode::OK, &e),
    }
}

async fn invoke_transaction(
    tx: UserInvokeFunctionTransaction,
    add_transaction_provider: Arc<dyn AddTransactionProvider>,
) -> Response<String> {
    match add_transaction_provider.add_invoke_transaction(tx.into()).await {
        Ok(result) => create_json_response(hyper::StatusCode::OK, &result),
        Err(e) => create_json_response(hyper::StatusCode::OK, &e),
    }
}
