use super::{
    error::GatewayError,
    helpers::{
        create_json_response, create_response_with_json_body, create_string_response, get_params_from_request,
        include_block_params,
    },
};
use crate::helpers::{block_view_from_params, not_found_response, view_from_params};
use anyhow::Context;
use bincode::Options;
use bytes::Buf;
use http_body_util::BodyExt;
use hyper::{body::Incoming, Request, Response, StatusCode};
use mc_db::MadaraBackend;
use mc_rpc::versions::user::v0_7_1::methods::trace::trace_block_transactions::trace_block_transactions_view as v0_7_1_trace_block_transactions;
use mc_submit_tx::{SubmitTransaction, SubmitValidatedTransaction};
use mp_class::{ClassInfo, ContractClass};
use mp_gateway::{
    block::ProviderBlockPreConfirmed,
    user_transaction::{
        AddTransactionResult, UserDeclareTransaction, UserDeployAccountTransaction, UserInvokeFunctionTransaction,
        UserTransaction,
    },
};
use mp_gateway::{
    block::{BlockStatus, ProviderBlock, ProviderBlockSignature},
    state_update::ProviderStateUpdate,
};
use mp_gateway::{
    error::{StarknetError, StarknetErrorCode},
    user_transaction::{AddDeclareTransactionResult, AddDeployAccountTransactionResult, AddInvokeTransactionResult},
};
use mp_rpc::v0_7_1::{BroadcastedDeclareTxn, TraceBlockTransactionsResult};
use mp_transactions::validated::ValidatedTransaction;
use serde::Serialize;
use serde_json::json;
use starknet_types_core::felt::Felt;
use std::sync::Arc;

pub async fn handle_get_preconfirmed_block(
    req: Request<Incoming>,
    backend: Arc<MadaraBackend>,
) -> Result<Response<String>, GatewayError> {
    let params = get_params_from_request(&req);
    let block_number = params.get("blockNumber").ok_or_else(|| {
        StarknetError::new(StarknetErrorCode::MalformedRequest, "Field blockNumber is required.".into())
    })?;

    let block_number: u64 = block_number
        .parse()
        .map_err(|e: std::num::ParseIntError| StarknetError::new(StarknetErrorCode::MalformedRequest, e.to_string()))?;

    let mut block =
        backend.block_view_on_preconfirmed().filter(|block| block.block_number() == block_number).ok_or_else(|| {
            StarknetError::new(
                StarknetErrorCode::BlockNotFound,
                format!("Pre-confirmed block with number {block_number} was not found."),
            )
        })?;

    block.refresh_with_candidates(); // We want candidates too :)
    let block = {
        let content = block.borrow_content();
        ProviderBlockPreConfirmed::new(
            block.header(),
            content.executed_transactions().map(|tx| (&tx.transaction, &tx.state_diff)),
            block.candidate_transactions().iter().map(|tx| &**tx),
            BlockStatus::PreConfirmed,
        )
    };

    Ok(create_json_response(hyper::StatusCode::OK, &block))
}

pub async fn handle_get_block(
    req: Request<Incoming>,
    backend: Arc<MadaraBackend>,
) -> Result<Response<String>, GatewayError> {
    let params = get_params_from_request(&req);
    let block = block_view_from_params(&backend, &params)?;

    if params.get("headerOnly").map(|s| s.as_ref()) == Some("true") {
        let Some(confirmed) = block.as_confirmed() else {
            return Err(StarknetError::no_block_header_for_pending_block().into());
        };

        let block_info = confirmed.get_block_info()?;

        let body = json!({
            "block_hash": block_info.block_hash,
            "block_number": block_info.header.block_number
        });
        Ok(create_json_response(hyper::StatusCode::OK, &body))
    } else {
        // Pending(preconfirmed) blocks can't be returned anymore from this endpoint
        let Some(block) = block.as_confirmed() else {
            return Err(StarknetError::block_not_found().into());
        };

        let status = if block.is_on_l1() { BlockStatus::AcceptedOnL1 } else { BlockStatus::AcceptedOnL2 };

        let info = block.get_block_info()?;
        let block_provider =
            ProviderBlock::new(info.block_hash, info.header, block.get_executed_transactions(..)?, status);
        Ok(create_json_response(hyper::StatusCode::OK, &block_provider))
    }
}

pub async fn handle_get_signature(
    req: Request<Incoming>,
    backend: Arc<MadaraBackend>,
) -> Result<Response<String>, GatewayError> {
    let params = get_params_from_request(&req);

    let block = block_view_from_params(&backend, &params)?;

    let Some(confirmed) = block.as_confirmed() else {
        return Err(StarknetError::no_signature_for_pending_block().into());
    };

    let block_info = confirmed.get_block_info()?;

    let private_key = &backend.chain_config().private_key;
    let signature = private_key.sign(&block_info.block_hash).context("Failed to sign block hash")?;
    let signature =
        ProviderBlockSignature { block_hash: block_info.block_hash, signature: vec![signature.r, signature.s] };
    Ok(create_json_response(hyper::StatusCode::OK, &signature))
}

pub async fn handle_get_state_update(
    req: Request<Incoming>,
    backend: Arc<MadaraBackend>,
) -> Result<Response<String>, GatewayError> {
    let params = get_params_from_request(&req);

    let block = block_view_from_params(&backend, &params)?;

    let Some(block) = block.as_confirmed() else {
        return Err(StarknetError::block_not_found().into());
    };

    let block_info = block.get_block_info()?;
    let state_update = ProviderStateUpdate {
        block_hash: block_info.block_hash,
        old_root: if let Some(parent_view) = block.parent_block() {
            parent_view.get_block_info()?.header.global_state_root
        } else {
            Felt::ZERO
        },
        new_root: block_info.header.global_state_root,
        state_diff: block.get_state_diff()?.into(),
    };

    let json_response = if include_block_params(&params) {
        let status = if block.is_on_l1() { BlockStatus::AcceptedOnL1 } else { BlockStatus::AcceptedOnL2 };
        let block_provider =
            ProviderBlock::new(block_info.block_hash, block_info.header, block.get_executed_transactions(..)?, status);

        create_json_response(hyper::StatusCode::OK, &json!({"block": block_provider, "state_update": state_update}))
    } else {
        create_json_response(hyper::StatusCode::OK, &state_update)
    };

    Ok(json_response)
}

pub async fn handle_get_block_traces(
    req: Request<Incoming>,
    backend: Arc<MadaraBackend>,
) -> Result<Response<String>, GatewayError> {
    let params = get_params_from_request(&req);
    let block = block_view_from_params(&backend, &params)?;

    #[derive(Serialize)]
    struct BlockTraces {
        traces: Vec<TraceBlockTransactionsResult>,
    }

    let traces = v0_7_1_trace_block_transactions(&block).await?;
    let block_traces = BlockTraces { traces };

    Ok(create_json_response(hyper::StatusCode::OK, &block_traces))
}

pub async fn handle_get_class_by_hash(
    req: Request<Incoming>,
    backend: Arc<MadaraBackend>,
) -> Result<Response<String>, GatewayError> {
    let params = get_params_from_request(&req);

    let view = view_from_params(&backend, &params)?;

    let class_hash = params.get("classHash").ok_or(StarknetError::missing_class_hash())?;
    let class_hash = Felt::from_hex(class_hash).map_err(StarknetError::invalid_class_hash)?;

    let class_info = view.get_class_info(&class_hash)?.ok_or(StarknetError::class_not_found(class_hash))?;

    let json_response = match class_info.contract_class() {
        ContractClass::Sierra(flattened_sierra_class) => {
            create_json_response(hyper::StatusCode::OK, flattened_sierra_class.as_ref())
        }
        ContractClass::Legacy(compressed_legacy_contract_class) => {
            let class = compressed_legacy_contract_class
                .as_ref()
                .serialize_to_json()
                .context("Failed to serialize legacy class")?;
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
    let view = view_from_params(&backend, &params)?;

    let class_hash = params.get("classHash").ok_or(StarknetError::missing_class_hash())?;
    let class_hash = Felt::from_hex(class_hash).map_err(StarknetError::invalid_class_hash)?;

    let class_info = view.get_class_info(&class_hash)?.ok_or(StarknetError::class_not_found(class_hash))?;

    let compiled_class_hash = match class_info {
        ClassInfo::Sierra(class_info) => class_info.compiled_class_hash,
        ClassInfo::Legacy(_) => {
            return Err(GatewayError::StarknetError(StarknetError::sierra_class_not_found(class_hash)))
        }
    };

    let class_compiled =
        view.get_class_compiled(&compiled_class_hash)?.ok_or(StarknetError::class_not_found(class_hash))?;

    Ok(create_response_with_json_body(hyper::StatusCode::OK, class_compiled.0.clone()))
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

pub async fn handle_add_validated_transaction(
    req: Request<Incoming>,
    submit_validated: Option<Arc<dyn SubmitValidatedTransaction>>,
) -> Result<Response<String>, GatewayError> {
    let Some(submit_validated) = submit_validated else { return Ok(not_found_response()) };
    let whole_body = req.collect().await.context("Failed to read request body")?.aggregate();

    let transaction: ValidatedTransaction = bincode::options()
        .with_little_endian()
        .deserialize_from(whole_body.reader())
        .map_err(|e| GatewayError::StarknetError(StarknetError::malformed_request(e)))?; // Fixed endinaness is important.

    submit_validated.submit_validated_transaction(transaction).await?;

    Ok(Response::builder().status(StatusCode::OK).body(String::new()).context("Building response")?)
}

pub async fn handle_add_transaction(
    req: Request<Incoming>,
    add_transaction_provider: Arc<dyn SubmitTransaction>,
) -> Result<Response<String>, GatewayError> {
    let whole_body = req.collect().await.context("Failed to read request body")?.aggregate();

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
    add_transaction_provider: Arc<dyn SubmitTransaction>,
) -> Response<String> {
    let tx: BroadcastedDeclareTxn = match tx.try_into() {
        Ok(tx) => tx,
        Err(e) => {
            let error = StarknetError::new(StarknetErrorCode::InvalidContractDefinition, e.to_string());
            return create_json_response(hyper::StatusCode::OK, &error);
        }
    };

    match add_transaction_provider.submit_declare_transaction(tx).await {
        Ok(result) => create_json_response(
            hyper::StatusCode::OK,
            &AddTransactionResult::from(AddDeclareTransactionResult {
                class_hash: result.class_hash,
                transaction_hash: result.transaction_hash,
            }),
        ),
        Err(e) => GatewayError::from(e).into(),
    }
}

async fn deploy_account_transaction(
    tx: UserDeployAccountTransaction,
    add_transaction_provider: Arc<dyn SubmitTransaction>,
) -> Response<String> {
    match add_transaction_provider.submit_deploy_account_transaction(tx.into()).await {
        Ok(result) => create_json_response(
            hyper::StatusCode::OK,
            &AddTransactionResult::from(AddDeployAccountTransactionResult {
                address: result.contract_address,
                transaction_hash: result.transaction_hash,
            }),
        ),
        Err(e) => GatewayError::from(e).into(),
    }
}

async fn invoke_transaction(
    tx: UserInvokeFunctionTransaction,
    add_transaction_provider: Arc<dyn SubmitTransaction>,
) -> Response<String> {
    match add_transaction_provider.submit_invoke_transaction(tx.into()).await {
        Ok(result) => create_json_response(
            hyper::StatusCode::OK,
            &AddTransactionResult::from(AddInvokeTransactionResult { transaction_hash: result.transaction_hash }),
        ),
        Err(e) => GatewayError::from(e).into(),
    }
}
