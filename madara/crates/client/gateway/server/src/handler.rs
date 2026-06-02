use super::{
    error::GatewayError,
    helpers::{
        create_json_response, create_response_with_json_body, create_string_response, get_params_from_request,
        include_block_params,
    },
};
use crate::helpers::{block_view_from_params, not_found_response, view_from_params};
use crate::metrics::{
    add_transaction_error_code, add_transaction_error_code_from_gateway_error,
    add_transaction_error_code_from_submit_error, add_transaction_result, add_transaction_result_from_gateway_error,
    add_transaction_result_from_submit_error, add_transaction_tx_type, metrics,
};
use anyhow::Context;
use bincode::Options;
use bytes::Buf;
use flate2::read::GzDecoder;
use http_body_util::BodyExt;
use hyper::header::{HeaderMap, CONTENT_ENCODING, CONTENT_TYPE};
use hyper::{body::Incoming, Request, Response, StatusCode};
use mc_db::MadaraBackend;
use mc_rpc::versions::user::v0_9_0::methods::trace::trace_block_transactions::trace_block_transactions_view as v0_9_0_trace_block_transactions;
use mc_submit_tx::{SubmitTransaction, SubmitTransactionError, SubmitValidatedTransaction};
use mp_block::MadaraMaybePreconfirmedBlockInfo;
use mp_class::{convert::ReadSizeLimiter, ClassInfo, ContractClass};
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
use mp_rpc::v0_9_0::{BroadcastedDeclareTxn, TraceBlockTransactionsResult};
use mp_transactions::validated::ValidatedTransaction;
use serde::Serialize;
use serde_json::json;
use starknet_types_core::felt::Felt;
use std::{borrow::Cow, io::Read, sync::Arc, time::Duration, time::Instant};

const GZIP_MAGIC_BYTES: [u8; 2] = [0x1f, 0x8b];
const BODY_PREFIX_HEX_BYTES: usize = 8;
// Match the default RPC request size until the gateway has dedicated body-size configuration.
const MAX_DECOMPRESSED_ADD_TRANSACTION_BODY_BYTES: u64 = 15 * 1024 * 1024;

fn is_gzip_encoded(headers: &HeaderMap) -> bool {
    headers
        .get(CONTENT_ENCODING)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.split(',').any(|encoding| encoding.trim().eq_ignore_ascii_case("gzip")))
        .unwrap_or(false)
}

fn log_add_transaction_request_failure(
    path: &str,
    headers: &HeaderMap,
    body: &[u8],
    error_code: &'static str,
    error: &dyn std::fmt::Display,
) {
    let content_type = headers.get(CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("<missing>").to_owned();
    let content_encoding =
        headers.get(CONTENT_ENCODING).and_then(|v| v.to_str().ok()).unwrap_or("<missing>").to_owned();
    let first_bytes: String =
        body.iter().take(BODY_PREFIX_HEX_BYTES).map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ");

    tracing::warn!(
        target: "gateway_errors",
        service = "gateway",
        endpoint = "add_transaction",
        tx_type = add_transaction_tx_type::UNKNOWN,
        error_code,
        request_path = path,
        content_type,
        content_encoding,
        body_len = body.len(),
        first_bytes_hex = first_bytes,
        looks_gzip = body.starts_with(&GZIP_MAGIC_BYTES),
        error = %error,
        "Failed to process gateway/add_transaction request body"
    );
}

fn log_gateway_add_transaction_error(tx_type: &'static str, error: &GatewayError, duration: Duration) {
    let error_code = add_transaction_error_code_from_gateway_error(error);
    let duration_ms = duration.as_secs_f64() * 1000.0;

    match error {
        GatewayError::StarknetError(error) => tracing::warn!(
            target: "gateway_errors",
            service = "gateway",
            endpoint = "add_transaction",
            tx_type,
            result = add_transaction_result::REJECTED,
            error_code,
            duration_ms,
            message = %error.message,
            "Gateway add_transaction rejected"
        ),
        GatewayError::InternalServerError => tracing::error!(
            target: "gateway_errors",
            service = "gateway",
            endpoint = "add_transaction",
            tx_type,
            result = add_transaction_result::INTERNAL_ERROR,
            error_code,
            duration_ms,
            "Gateway add_transaction failed with an internal server error"
        ),
        GatewayError::Unsupported => tracing::warn!(
            target: "gateway_errors",
            service = "gateway",
            endpoint = "add_transaction",
            tx_type,
            result = add_transaction_result::UNSUPPORTED,
            error_code,
            duration_ms,
            "Gateway add_transaction is unsupported"
        ),
    }
}

fn log_submit_transaction_error(tx_type: &'static str, error: &SubmitTransactionError, duration: Duration) {
    let error_code = add_transaction_error_code_from_submit_error(error);
    let result = add_transaction_result_from_submit_error(error);
    let duration_ms = duration.as_secs_f64() * 1000.0;

    match error {
        SubmitTransactionError::Rejected(error) => tracing::warn!(
            target: "gateway_errors",
            service = "gateway",
            endpoint = "add_transaction",
            tx_type,
            result,
            error_code,
            duration_ms,
            error = %error,
            "Gateway add_transaction rejected"
        ),
        SubmitTransactionError::Internal(error) => tracing::error!(
            target: "gateway_errors",
            service = "gateway",
            endpoint = "add_transaction",
            tx_type,
            result,
            error_code,
            duration_ms,
            error = %error,
            "Gateway add_transaction failed"
        ),
        SubmitTransactionError::Unsupported => tracing::warn!(
            target: "gateway_errors",
            service = "gateway",
            endpoint = "add_transaction",
            tx_type,
            result,
            error_code,
            duration_ms,
            "Gateway add_transaction is unsupported"
        ),
    }
}

fn record_gateway_error(tx_type: &'static str, error: &GatewayError, duration: Duration) {
    metrics().record_add_transaction(
        tx_type,
        add_transaction_result_from_gateway_error(error),
        add_transaction_error_code_from_gateway_error(error),
        duration,
    );
    log_gateway_add_transaction_error(tx_type, error, duration);
}

fn record_submit_transaction_error(tx_type: &'static str, error: &SubmitTransactionError, duration: Duration) {
    metrics().record_add_transaction(
        tx_type,
        add_transaction_result_from_submit_error(error),
        add_transaction_error_code_from_submit_error(error),
        duration,
    );
    log_submit_transaction_error(tx_type, error, duration);
}

fn record_add_transaction_success(tx_type: &'static str, duration: Duration) {
    metrics().record_add_transaction(
        tx_type,
        add_transaction_result::SUCCESS,
        add_transaction_error_code::NONE,
        duration,
    );
}

fn decode_gzip_request_body(raw_body: &[u8], max_decompressed_body_bytes: u64) -> Result<Vec<u8>, std::io::Error> {
    let mut decoder = ReadSizeLimiter::new(GzDecoder::new(raw_body), max_decompressed_body_bytes);
    let mut decoded_body = Vec::new();
    decoder.read_to_end(&mut decoded_body)?;
    Ok(decoded_body)
}

fn parse_add_transaction_request(
    path: &str,
    headers: &HeaderMap,
    raw_body: &[u8],
) -> Result<UserTransaction, GatewayError> {
    parse_add_transaction_request_with_max_body_size(
        path,
        headers,
        raw_body,
        MAX_DECOMPRESSED_ADD_TRANSACTION_BODY_BYTES,
    )
}

fn parse_add_transaction_request_with_max_body_size(
    path: &str,
    headers: &HeaderMap,
    raw_body: &[u8],
    max_decompressed_body_bytes: u64,
) -> Result<UserTransaction, GatewayError> {
    let decoded_body = if is_gzip_encoded(headers) {
        let decoded_body = decode_gzip_request_body(raw_body, max_decompressed_body_bytes).map_err(|error| {
            log_add_transaction_request_failure(path, headers, raw_body, "malformed_request", &error);
            GatewayError::StarknetError(StarknetError::new(
                StarknetErrorCode::MalformedRequest,
                format!("Failed to decode gzip request body: {error}"),
            ))
        })?;
        Cow::Owned(decoded_body)
    } else {
        Cow::Borrowed(raw_body)
    };

    serde_json::from_slice::<UserTransaction>(decoded_body.as_ref()).map_err(|error| {
        log_add_transaction_request_failure(path, headers, raw_body, "malformed_request", &error);
        GatewayError::StarknetError(StarknetError::malformed_request(error))
    })
}

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

    // Use block_view_on_preconfirmed_or_fake() - this always returns a block
    let mut block = backend
        .block_view_on_preconfirmed_or_fake()
        .map_err(|e| StarknetError::new(StarknetErrorCode::BlockNotFound, e.to_string()))?;

    // Check if the requested block number matches the pre-confirmed block number
    if block.block_number() != block_number {
        return Err(StarknetError::new(
            StarknetErrorCode::BlockNotFound,
            format!("Pre-confirmed block with number {block_number} was not found. Current pre-confirmed block number is {}.",
                block.block_number()),
        ).into());
    }

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

pub async fn handle_get_block_bouncer_config(
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

    let block = backend.block_view_on_confirmed(block_number).ok_or_else(|| {
        StarknetError::new(
            StarknetErrorCode::BlockNotFound,
            format!("Pre-confirmed block with number {block_number} was not found."),
        )
    })?;

    let bouncer_weights = block.get_bouncer_weights()?;

    Ok(create_json_response(hyper::StatusCode::OK, &bouncer_weights))
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
        let info = block.get_block_info()?;
        let txs = block.get_executed_transactions(..)?;

        match info {
            MadaraMaybePreconfirmedBlockInfo::Confirmed(info) => {
                let status = if block.is_on_l1() { BlockStatus::AcceptedOnL1 } else { BlockStatus::AcceptedOnL2 };
                let block_provider = ProviderBlock::new(info.block_hash, info.header, txs, status);
                Ok(create_json_response(hyper::StatusCode::OK, &block_provider))
            }
            MadaraMaybePreconfirmedBlockInfo::Preconfirmed(_) => {
                // TODO(@bytezorvin, 2025-12-09): Return preconfirmed block when starknet.rs adds
                // support for it and migrates feeder gateway to 0.9.0 RPC version from 0.8.1.
                //
                // Currently returning the last confirmed block because starknet.rs
                // SequencerGatewayProvider does NOT support pending/preconfirmed block conversion.
                // Use get_preconfirmed_block endpoint for madara's preconfirmed block info.
                let confirmed = backend.block_view_on_last_confirmed().ok_or_else(StarknetError::block_not_found)?;
                let info = confirmed.get_block_info()?;
                let txs = confirmed.get_executed_transactions(..)?;
                let status = if confirmed.is_on_l1() { BlockStatus::AcceptedOnL1 } else { BlockStatus::AcceptedOnL2 };
                let block_provider = ProviderBlock::new(info.block_hash, info.header, txs, status);
                Ok(create_json_response(hyper::StatusCode::OK, &block_provider))
            }
        }
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

    let private_key = backend.chain_config().private_key.as_ref().context("Private key not available for signing")?;
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

    let traces = v0_9_0_trace_block_transactions(&block).await?;
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

    let compiled_class_hash = match &class_info {
        ClassInfo::Sierra(_) => class_info.compiled_class_hash().ok_or_else(|| {
            tracing::error!("Sierra class {class_hash:#x} is missing compiled_class_hash - database inconsistency");
            GatewayError::InternalServerError
        })?,
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
            "GpsStatementVerifier": chain_config.eth_gps_statement_verifier,
            "eth_l2_token_address": chain_config.parent_fee_token_address,
            "strk_l2_token_address": chain_config.native_fee_token_address,
        }),
    ))
}

pub async fn handle_get_public_key(backend: Arc<MadaraBackend>) -> Result<Response<String>, GatewayError> {
    let public_key =
        backend.chain_config().private_key.as_ref().map(|pk| pk.public).context("Public key not available")?;
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
    let started_at = Instant::now();
    let path = req.uri().path().to_owned();
    let headers = req.headers().clone();
    let mut whole_body = req.collect().await.context("Failed to read request body")?.aggregate();
    let body_len = whole_body.remaining();
    let whole_body = whole_body.copy_to_bytes(body_len);

    let transaction = match parse_add_transaction_request(&path, &headers, whole_body.as_ref()) {
        Ok(transaction) => transaction,
        Err(error) => {
            record_gateway_error(add_transaction_tx_type::UNKNOWN, &error, started_at.elapsed());
            return Err(error);
        }
    };

    let response = match transaction {
        UserTransaction::Declare(tx) => declare_transaction(tx, add_transaction_provider, started_at).await,
        UserTransaction::DeployAccount(tx) => {
            deploy_account_transaction(tx, add_transaction_provider, started_at).await
        }
        UserTransaction::InvokeFunction(tx) => invoke_transaction(tx, add_transaction_provider, started_at).await,
    };

    Ok(response)
}

async fn declare_transaction(
    tx: UserDeclareTransaction,
    add_transaction_provider: Arc<dyn SubmitTransaction>,
    started_at: Instant,
) -> Response<String> {
    let tx: BroadcastedDeclareTxn = match tx.try_into() {
        Ok(tx) => tx,
        Err(e) => {
            let error = StarknetError::new(StarknetErrorCode::InvalidContractDefinition, e.to_string());
            let error = GatewayError::StarknetError(error);
            let duration = started_at.elapsed();
            record_gateway_error(add_transaction_tx_type::DECLARE, &error, duration);
            return error.into();
        }
    };

    match add_transaction_provider.submit_declare_transaction(tx).await {
        Ok(result) => {
            record_add_transaction_success(add_transaction_tx_type::DECLARE, started_at.elapsed());
            create_json_response(
                hyper::StatusCode::OK,
                &AddTransactionResult::from(AddDeclareTransactionResult {
                    class_hash: result.class_hash,
                    transaction_hash: result.transaction_hash,
                }),
            )
        }
        Err(error) => {
            let duration = started_at.elapsed();
            record_submit_transaction_error(add_transaction_tx_type::DECLARE, &error, duration);
            GatewayError::from_submit_transaction_error_unlogged(error).into()
        }
    }
}

async fn deploy_account_transaction(
    tx: UserDeployAccountTransaction,
    add_transaction_provider: Arc<dyn SubmitTransaction>,
    started_at: Instant,
) -> Response<String> {
    match add_transaction_provider.submit_deploy_account_transaction(tx.into()).await {
        Ok(result) => {
            record_add_transaction_success(add_transaction_tx_type::DEPLOY_ACCOUNT, started_at.elapsed());
            create_json_response(
                hyper::StatusCode::OK,
                &AddTransactionResult::from(AddDeployAccountTransactionResult {
                    address: result.contract_address,
                    transaction_hash: result.transaction_hash,
                }),
            )
        }
        Err(error) => {
            let duration = started_at.elapsed();
            record_submit_transaction_error(add_transaction_tx_type::DEPLOY_ACCOUNT, &error, duration);
            GatewayError::from_submit_transaction_error_unlogged(error).into()
        }
    }
}

async fn invoke_transaction(
    tx: UserInvokeFunctionTransaction,
    add_transaction_provider: Arc<dyn SubmitTransaction>,
    started_at: Instant,
) -> Response<String> {
    match add_transaction_provider.submit_invoke_transaction(tx.into()).await {
        Ok(result) => {
            record_add_transaction_success(add_transaction_tx_type::INVOKE, started_at.elapsed());
            create_json_response(
                hyper::StatusCode::OK,
                &AddTransactionResult::from(AddInvokeTransactionResult { transaction_hash: result.transaction_hash }),
            )
        }
        Err(error) => {
            let duration = started_at.elapsed();
            record_submit_transaction_error(add_transaction_tx_type::INVOKE, &error, duration);
            GatewayError::from_submit_transaction_error_unlogged(error).into()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{write::GzEncoder, Compression};
    use hyper::header::HeaderValue;
    use rstest::rstest;
    use std::io::Write;

    const TEST_PATH: &str = "/gateway/add_transaction";

    fn request_headers(content_encoding: Option<&'static str>) -> HeaderMap {
        let mut headers = HeaderMap::new();
        if let Some(content_encoding) = content_encoding {
            headers.insert(CONTENT_ENCODING, HeaderValue::from_static(content_encoding));
        }
        headers
    }

    fn invoke_transaction_body(calldata_len: usize) -> Vec<u8> {
        let calldata = vec!["0x1"; calldata_len];
        serde_json::to_vec(&serde_json::json!({
            "type": "INVOKE_FUNCTION",
            "version": "0x1",
            "sender_address": "0x1",
            "calldata": calldata,
            "signature": ["0x2"],
            "max_fee": "0x0",
            "nonce": "0x0"
        }))
        .expect("valid invoke transaction body")
    }

    fn gzip_body(body: &[u8]) -> Vec<u8> {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(body).expect("write gzip body");
        encoder.finish().expect("finalize gzip body")
    }

    fn malformed_request(error: GatewayError) -> StarknetError {
        match error {
            GatewayError::StarknetError(error) => {
                assert_eq!(error.code, StarknetErrorCode::MalformedRequest);
                error
            }
            error => panic!("expected malformed request error, got {error:?}"),
        }
    }

    #[rstest]
    #[case(None)]
    #[case(Some("gzip"))]
    fn parse_add_transaction_request_accepts_plain_and_gzip_bodies(#[case] content_encoding: Option<&'static str>) {
        let raw_body = invoke_transaction_body(1);
        let headers = request_headers(content_encoding);
        let body = if content_encoding.is_some() { gzip_body(&raw_body) } else { raw_body };

        let transaction = parse_add_transaction_request(TEST_PATH, &headers, &body).expect("body should parse");

        assert!(matches!(transaction, UserTransaction::InvokeFunction(UserInvokeFunctionTransaction::V1(_))));
    }

    #[test]
    fn parse_add_transaction_request_rejects_invalid_gzip_body() {
        let headers = request_headers(Some("gzip"));
        let error = parse_add_transaction_request(TEST_PATH, &headers, br#"{"not":"gzip"}"#)
            .expect_err("gzip body should fail");
        let error = malformed_request(error);

        assert!(error.message.starts_with("Failed to decode gzip request body:"));
    }

    #[test]
    fn parse_add_transaction_request_rejects_oversized_gzip_body() {
        let raw_body = invoke_transaction_body(64);
        let headers = request_headers(Some("gzip"));
        let body = gzip_body(&raw_body);
        let error = parse_add_transaction_request_with_max_body_size(TEST_PATH, &headers, &body, 64)
            .expect_err("oversized decompressed body should fail");
        let error = malformed_request(error);

        assert!(error.message.contains("Read input is too large"));
    }
}
