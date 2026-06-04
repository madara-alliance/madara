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
use flate2::read::GzDecoder;
use http_body_util::BodyExt;
use hyper::header::{HeaderMap, CONTENT_ENCODING, CONTENT_TYPE};
use hyper::{body::Incoming, Request, Response, StatusCode};
use mc_db::MadaraBackend;
use mc_rpc::versions::user::v0_9_0::methods::trace::trace_block_transactions::trace_block_transactions_view as v0_9_0_trace_block_transactions;
use mc_submit_tx::{
    feeder_status_from_backend_view, feeder_transaction_from_backend_view, SubmitTransaction,
    SubmitValidatedTransaction, TransactionLookup,
};
use mp_block::MadaraMaybePreconfirmedBlockInfo;
use mp_class::{convert::ReadSizeLimiter, ClassInfo, ContractClass};
use mp_gateway::{
    block::ProviderBlockPreConfirmed,
    feeder::{ProviderTransactionResponse, ProviderTransactionStatus, TransactionExecutionStatus, TransactionStatus},
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
use std::{borrow::Cow, io::Read, sync::Arc};

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

fn log_add_transaction_failure(path: &str, headers: &HeaderMap, body: &[u8], error: &dyn std::fmt::Display) {
    let content_type = headers.get(CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("<missing>").to_owned();
    let content_encoding =
        headers.get(CONTENT_ENCODING).and_then(|v| v.to_str().ok()).unwrap_or("<missing>").to_owned();
    let first_bytes: String =
        body.iter().take(BODY_PREFIX_HEX_BYTES).map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ");

    tracing::warn!(
        target: "gateway_errors",
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
            log_add_transaction_failure(path, headers, raw_body, &error);
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
        log_add_transaction_failure(path, headers, raw_body, &error);
        GatewayError::StarknetError(StarknetError::malformed_request(error))
    })
}

fn parse_transaction_hash(params: &std::collections::HashMap<String, String>) -> Result<Felt, GatewayError> {
    let transaction_hash = params.get("transactionHash").ok_or_else(|| {
        StarknetError::new(StarknetErrorCode::MalformedRequest, "Field transactionHash is required.".into())
    })?;

    Felt::from_hex(transaction_hash)
        .map_err(|e| StarknetError::new(StarknetErrorCode::MalformedRequest, e.to_string()).into())
}

fn parse_block_hash(params: &std::collections::HashMap<String, String>) -> Result<Felt, GatewayError> {
    let block_hash = params.get("blockHash").ok_or_else(|| {
        StarknetError::new(StarknetErrorCode::MalformedRequest, "Field blockHash is required.".into())
    })?;

    Felt::from_hex(block_hash)
        .map_err(|e| StarknetError::new(StarknetErrorCode::MalformedRequest, e.to_string()).into())
}

fn parse_block_id(params: &std::collections::HashMap<String, String>) -> Result<u64, GatewayError> {
    let block_id = params
        .get("blockId")
        .ok_or_else(|| StarknetError::new(StarknetErrorCode::MalformedRequest, "Field blockId is required.".into()))?;

    block_id.parse().map_err(|e: std::num::ParseIntError| {
        StarknetError::new(StarknetErrorCode::MalformedRequest, e.to_string()).into()
    })
}

async fn transaction_status_response(
    transaction_hash: Felt,
    backend: &Arc<MadaraBackend>,
    transaction_lookup: &Arc<dyn TransactionLookup>,
) -> Result<ProviderTransactionStatus, GatewayError> {
    let view = backend.view_on_latest();

    if let Some(res) = view.find_transaction_by_hash(&transaction_hash)? {
        Ok(feeder_status_from_backend_view(&res)?)
    } else if let Some(status) = transaction_lookup.feeder_transaction_status(transaction_hash).await? {
        Ok(status)
    } else {
        Ok(ProviderTransactionStatus::not_received())
    }
}

async fn transaction_response(
    transaction_hash: Felt,
    backend: &Arc<MadaraBackend>,
    transaction_lookup: &Arc<dyn TransactionLookup>,
) -> Result<ProviderTransactionResponse, GatewayError> {
    let view = backend.view_on_latest();

    if let Some(res) = view.find_transaction_by_hash(&transaction_hash)? {
        Ok(feeder_transaction_from_backend_view(&res)?)
    } else if let Some(response) = transaction_lookup.feeder_transaction(transaction_hash).await? {
        Ok(response)
    } else {
        Ok(ProviderTransactionResponse::not_received())
    }
}

fn block_hash_by_id_response(block_id: u64, backend: &Arc<MadaraBackend>) -> Result<Felt, GatewayError> {
    let Some(latest_confirmed) = backend.latest_confirmed_block_n() else {
        return Err(StarknetError::block_not_found().into());
    };

    if block_id > latest_confirmed {
        return Err(StarknetError::new(
            StarknetErrorCode::MalformedRequest,
            format!("Block ID should be in the range [0, {}); got: {}.", latest_confirmed + 1, block_id),
        )
        .into());
    }

    let block = backend.block_view_on_confirmed(block_id).ok_or_else(StarknetError::block_not_found)?;
    Ok(block.get_block_info()?.block_hash)
}

fn block_id_by_hash_response(block_hash: Felt, backend: &Arc<MadaraBackend>) -> Result<u64, GatewayError> {
    let Some(latest_confirmed) = backend.latest_confirmed_block_n() else {
        return Err(StarknetError::block_not_found().into());
    };

    let block_id = backend.view_on_latest().find_block_by_hash(&block_hash)?.ok_or_else(|| {
        StarknetError::new(StarknetErrorCode::BlockNotFound, format!("Block hash {block_hash:#x} does not exist."))
    })?;

    if block_id > latest_confirmed {
        return Err(StarknetError::new(
            StarknetErrorCode::BlockNotFound,
            format!("Block hash {block_hash:#x} does not exist."),
        )
        .into());
    }

    Ok(block_id)
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

pub async fn handle_get_transaction(
    req: Request<Incoming>,
    backend: Arc<MadaraBackend>,
    transaction_lookup: Arc<dyn TransactionLookup>,
) -> Result<Response<String>, GatewayError> {
    let params = get_params_from_request(&req);
    let transaction_hash = parse_transaction_hash(&params)?;
    let response = transaction_response(transaction_hash, &backend, &transaction_lookup).await?;
    Ok(create_json_response(hyper::StatusCode::OK, &response))
}

pub async fn handle_get_transaction_status(
    req: Request<Incoming>,
    backend: Arc<MadaraBackend>,
    transaction_lookup: Arc<dyn TransactionLookup>,
) -> Result<Response<String>, GatewayError> {
    let params = get_params_from_request(&req);
    let transaction_hash = parse_transaction_hash(&params)?;
    let response = transaction_status_response(transaction_hash, &backend, &transaction_lookup).await?;
    Ok(create_json_response(hyper::StatusCode::OK, &response))
}

pub async fn handle_get_block_hash_by_id(
    req: Request<Incoming>,
    backend: Arc<MadaraBackend>,
) -> Result<Response<String>, GatewayError> {
    let params = get_params_from_request(&req);
    let block_id = parse_block_id(&params)?;
    let block_hash = block_hash_by_id_response(block_id, &backend)?;
    Ok(create_json_response(hyper::StatusCode::OK, &block_hash))
}

pub async fn handle_get_block_id_by_hash(
    req: Request<Incoming>,
    backend: Arc<MadaraBackend>,
) -> Result<Response<String>, GatewayError> {
    let params = get_params_from_request(&req);
    let block_hash = parse_block_hash(&params)?;
    let block_id = block_id_by_hash_response(block_hash, &backend)?;
    Ok(create_json_response(hyper::StatusCode::OK, &block_id))
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
    transaction_submitter: Arc<dyn SubmitTransaction>,
) -> Result<Response<String>, GatewayError> {
    let path = req.uri().path().to_owned();
    let headers = req.headers().clone();
    let mut whole_body = req.collect().await.context("Failed to read request body")?.aggregate();
    let body_len = whole_body.remaining();
    let whole_body = whole_body.copy_to_bytes(body_len);

    let transaction = parse_add_transaction_request(&path, &headers, whole_body.as_ref())?;

    let response = match transaction {
        UserTransaction::Declare(tx) => declare_transaction(tx, transaction_submitter).await,
        UserTransaction::DeployAccount(tx) => deploy_account_transaction(tx, transaction_submitter).await,
        UserTransaction::InvokeFunction(tx) => invoke_transaction(tx, transaction_submitter).await,
    };

    Ok(response)
}

async fn declare_transaction(
    tx: UserDeclareTransaction,
    transaction_submitter: Arc<dyn SubmitTransaction>,
) -> Response<String> {
    let tx: BroadcastedDeclareTxn = match tx.try_into() {
        Ok(tx) => tx,
        Err(e) => {
            let error = StarknetError::new(StarknetErrorCode::InvalidContractDefinition, e.to_string());
            return GatewayError::StarknetError(error).into();
        }
    };

    match transaction_submitter.submit_declare_transaction(tx).await {
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
    transaction_submitter: Arc<dyn SubmitTransaction>,
) -> Response<String> {
    match transaction_submitter.submit_deploy_account_transaction(tx.into()).await {
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
    transaction_submitter: Arc<dyn SubmitTransaction>,
) -> Response<String> {
    match transaction_submitter.submit_invoke_transaction(tx.into()).await {
        Ok(result) => create_json_response(
            hyper::StatusCode::OK,
            &AddTransactionResult::from(AddInvokeTransactionResult { transaction_hash: result.transaction_hash }),
        ),
        Err(e) => GatewayError::from(e).into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use flate2::{write::GzEncoder, Compression};
    use hyper::header::HeaderValue;
    use mc_submit_tx::{SubmitTransactionError, TransactionValidatorConfig};
    use mp_receipt::ExecutionResult;
    use rstest::rstest;
    use std::io::Write;

    const TEST_PATH: &str = "/gateway/add_transaction";
    const TX_HASH: Felt = starknet_types_core::felt::Felt::from_hex_unchecked(
        "0x3ccaabf599097d1965e1ef8317b830e76eb681016722c9364ed6e59f3252908",
    );

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

    fn backend_for_tests() -> Arc<MadaraBackend> {
        let chain_config = Arc::new(mp_chain_config::ChainConfig::madara_test());
        mc_db::MadaraBackend::open_for_testing(chain_config)
    }

    #[derive(Clone)]
    struct StubSubmitTransaction {
        status: Option<ProviderTransactionStatus>,
        transaction: Option<ProviderTransactionResponse>,
    }

    impl StubSubmitTransaction {
        fn with_status(status: ProviderTransactionStatus) -> Arc<dyn TransactionLookup> {
            Arc::new(Self { status: Some(status), transaction: None })
        }

        fn with_transaction(transaction: ProviderTransactionResponse) -> Arc<dyn TransactionLookup> {
            Arc::new(Self { status: None, transaction: Some(transaction) })
        }
    }

    #[async_trait]
    impl SubmitTransaction for StubSubmitTransaction {
        async fn submit_declare_transaction(
            &self,
            _tx: mp_rpc::v0_9_0::BroadcastedDeclareTxn,
        ) -> Result<mp_rpc::v0_9_0::ClassAndTxnHash, SubmitTransactionError> {
            Err(SubmitTransactionError::Unsupported)
        }

        async fn submit_deploy_account_transaction(
            &self,
            _tx: mp_rpc::v0_9_0::BroadcastedDeployAccountTxn,
        ) -> Result<mp_rpc::v0_9_0::ContractAndTxnHash, SubmitTransactionError> {
            Err(SubmitTransactionError::Unsupported)
        }

        async fn submit_invoke_transaction(
            &self,
            _tx: mp_rpc::v0_10_2::BroadcastedInvokeTxn,
        ) -> Result<mp_rpc::v0_9_0::AddInvokeTransactionResult, SubmitTransactionError> {
            Err(SubmitTransactionError::Unsupported)
        }
    }

    #[async_trait]
    impl TransactionLookup for StubSubmitTransaction {
        async fn received_transaction(&self, _hash: Felt) -> Option<bool> {
            None
        }

        async fn subscribe_new_transactions(&self) -> Option<tokio::sync::broadcast::Receiver<Felt>> {
            None
        }

        async fn feeder_transaction_status(
            &self,
            _hash: Felt,
        ) -> Result<Option<ProviderTransactionStatus>, SubmitTransactionError> {
            Ok(self.status.clone())
        }

        async fn feeder_transaction(
            &self,
            _hash: Felt,
        ) -> Result<Option<ProviderTransactionResponse>, SubmitTransactionError> {
            Ok(self.transaction.clone())
        }
    }

    fn submit_provider() -> (Arc<MadaraBackend>, Arc<dyn SubmitTransaction>, Arc<dyn TransactionLookup>) {
        let backend = backend_for_tests();
        let mempool = Arc::new(mc_mempool::Mempool::new(Arc::clone(&backend), mc_mempool::MempoolConfig::default()));
        let validation = TransactionValidatorConfig { disable_validation: true, disable_fee: false };
        let provider = Arc::new(mc_submit_tx::TransactionValidator::new(mempool, Arc::clone(&backend), validation));

        (backend, Arc::clone(&provider) as _, provider as _)
    }

    fn submit_invoke_tx() -> mp_rpc::v0_10_2::BroadcastedInvokeTxn {
        mp_rpc::v0_10_2::BroadcastedInvokeTxn::V3(mp_rpc::v0_10_2::BroadcastedInvokeTxnV3 {
            inner: mp_rpc::v0_10_0::InvokeTxnV3 {
                calldata: Default::default(),
                sender_address: Default::default(),
                signature: Default::default(),
                nonce: Default::default(),
                resource_bounds: mp_rpc::v0_10_0::ResourceBoundsMapping {
                    l1_gas: mp_rpc::v0_10_0::ResourceBounds { max_amount: 0, max_price_per_unit: 0 },
                    l2_gas: mp_rpc::v0_10_0::ResourceBounds { max_amount: 0, max_price_per_unit: 0 },
                    l1_data_gas: mp_rpc::v0_10_0::ResourceBounds { max_amount: 0, max_price_per_unit: 0 },
                },
                tip: Default::default(),
                paymaster_data: Default::default(),
                account_deployment_data: Default::default(),
                nonce_data_availability_mode: mp_rpc::v0_10_0::DaMode::L1,
                fee_data_availability_mode: mp_rpc::v0_10_0::DaMode::L1,
            },
            proof: None,
            proof_facts: None,
        })
    }

    fn tx_with_receipt() -> mp_block::TransactionWithReceipt {
        let tx = mp_rpc::v0_9_0::BroadcastedInvokeTxn::V3(mp_rpc::v0_9_0::InvokeTxnV3 {
            calldata: Default::default(),
            sender_address: Default::default(),
            signature: Default::default(),
            nonce: Default::default(),
            resource_bounds: mp_rpc::v0_9_0::ResourceBoundsMapping {
                l1_gas: mp_rpc::v0_9_0::ResourceBounds { max_amount: 0, max_price_per_unit: 0 },
                l2_gas: mp_rpc::v0_9_0::ResourceBounds { max_amount: 0, max_price_per_unit: 0 },
                l1_data_gas: mp_rpc::v0_9_0::ResourceBounds { max_amount: 0, max_price_per_unit: 0 },
            },
            tip: Default::default(),
            paymaster_data: Default::default(),
            account_deployment_data: Default::default(),
            nonce_data_availability_mode: mp_rpc::v0_9_0::DaMode::L1,
            fee_data_availability_mode: mp_rpc::v0_9_0::DaMode::L1,
        });

        mp_block::TransactionWithReceipt {
            transaction: mp_transactions::Transaction::Invoke(tx.into()),
            receipt: mp_receipt::TransactionReceipt::Invoke(mp_receipt::InvokeTransactionReceipt {
                transaction_hash: TX_HASH,
                execution_result: ExecutionResult::Succeeded,
                ..Default::default()
            }),
        }
    }

    fn preconfirmed_tx() -> mc_db::preconfirmed::PreconfirmedExecutedTransaction {
        mc_db::preconfirmed::PreconfirmedExecutedTransaction {
            transaction: tx_with_receipt(),
            state_diff: Default::default(),
            declared_class: None,
            arrived_at: mp_transactions::validated::TxTimestamp::now(),
            paid_fee_on_l1: None,
        }
    }

    fn empty_block(block_number: u64) -> mp_block::FullBlockWithoutCommitments {
        mp_block::FullBlockWithoutCommitments {
            header: mp_block::header::PreconfirmedHeader { block_number, ..Default::default() },
            state_diff: Default::default(),
            transactions: vec![],
            events: Default::default(),
        }
    }

    fn full_block(block_number: u64) -> mp_block::FullBlockWithoutCommitments {
        mp_block::FullBlockWithoutCommitments {
            header: mp_block::header::PreconfirmedHeader { block_number, ..Default::default() },
            state_diff: Default::default(),
            transactions: vec![tx_with_receipt()],
            events: Default::default(),
        }
    }

    #[tokio::test]
    async fn get_transaction_status_returns_not_received_for_unknown_hash() {
        let (backend, _, lookup) = submit_provider();

        let status = transaction_status_response(TX_HASH, &backend, &lookup).await.unwrap();

        assert_eq!(status, ProviderTransactionStatus::not_received());
    }

    #[tokio::test]
    async fn get_transaction_status_returns_not_received_when_in_mempool() {
        let (backend, submitter, lookup) = submit_provider();
        let tx_hash = submitter.submit_invoke_transaction(submit_invoke_tx()).await.unwrap().transaction_hash;

        let status = transaction_status_response(tx_hash, &backend, &lookup).await.unwrap();

        assert_eq!(status, ProviderTransactionStatus::not_received());
    }

    #[tokio::test]
    async fn get_transaction_status_uses_submit_provider_payload_when_backend_misses() {
        let backend = backend_for_tests();
        let expected = ProviderTransactionStatus::with_status(
            TransactionStatus::AcceptedOnL2,
            Some(TransactionExecutionStatus::Succeeded),
            Some(Felt::ONE),
            None,
        );
        let provider = StubSubmitTransaction::with_status(expected.clone());

        let status = transaction_status_response(TX_HASH, &backend, &provider).await.unwrap();

        assert_eq!(status, expected);
    }

    #[tokio::test]
    async fn get_transaction_status_returns_not_received_for_preconfirmed_backend_tx() {
        let backend = backend_for_tests();
        backend
            .write_access()
            .new_preconfirmed(mc_db::preconfirmed::PreconfirmedBlock::new_with_content(
                mp_block::header::PreconfirmedHeader { block_number: 0, ..Default::default() },
                [preconfirmed_tx()],
                std::iter::empty::<Arc<mp_transactions::validated::ValidatedTransaction>>(),
            ))
            .expect("Failed to persist preconfirmed block");
        let provider = StubSubmitTransaction::with_status(ProviderTransactionStatus::not_received());

        let status = transaction_status_response(TX_HASH, &backend, &provider).await.unwrap();

        assert_eq!(status, ProviderTransactionStatus::not_received());
    }

    #[tokio::test]
    async fn get_transaction_returns_confirmed_payload() {
        let (backend, submitter, lookup) = submit_provider();
        backend
            .write_access()
            .add_full_block_with_classes(&full_block(0), &[], true)
            .expect("Failed to persist confirmed block");

        let _ = submitter;
        let response = transaction_response(TX_HASH, &backend, &lookup).await.unwrap();

        assert_eq!(response.status, TransactionStatus::AcceptedOnL2);
        assert_eq!(response.finality_status, TransactionStatus::AcceptedOnL2);
        assert_eq!(response.execution_status, Some(TransactionExecutionStatus::Succeeded));
        assert_eq!(response.block_number, Some(0));
        assert_eq!(response.transaction_index, Some(0));
        assert!(response.block_hash.is_some());
        assert!(response.transaction.is_some());
    }

    #[tokio::test]
    async fn get_transaction_returns_not_received_when_in_mempool() {
        let (backend, submitter, lookup) = submit_provider();
        let tx_hash = submitter.submit_invoke_transaction(submit_invoke_tx()).await.unwrap().transaction_hash;

        let response = transaction_response(tx_hash, &backend, &lookup).await.unwrap();

        assert_eq!(response, ProviderTransactionResponse::not_received());
    }

    #[tokio::test]
    async fn get_transaction_uses_submit_provider_payload_when_backend_misses() {
        let backend = backend_for_tests();
        let expected = ProviderTransactionResponse::with_status(
            TransactionStatus::AcceptedOnL2,
            Some(TransactionExecutionStatus::Succeeded),
            Some(Felt::ONE),
            Some(7),
            Some(2),
            None,
        );
        let provider = StubSubmitTransaction::with_transaction(expected.clone());

        let response = transaction_response(TX_HASH, &backend, &provider).await.unwrap();

        assert_eq!(response, expected);
    }

    #[derive(Clone)]
    struct FailingSubmitTransaction;

    #[async_trait]
    impl SubmitTransaction for FailingSubmitTransaction {
        async fn submit_declare_transaction(
            &self,
            _tx: mp_rpc::v0_9_0::BroadcastedDeclareTxn,
        ) -> Result<mp_rpc::v0_9_0::ClassAndTxnHash, SubmitTransactionError> {
            Err(SubmitTransactionError::Unsupported)
        }

        async fn submit_deploy_account_transaction(
            &self,
            _tx: mp_rpc::v0_9_0::BroadcastedDeployAccountTxn,
        ) -> Result<mp_rpc::v0_9_0::ContractAndTxnHash, SubmitTransactionError> {
            Err(SubmitTransactionError::Unsupported)
        }

        async fn submit_invoke_transaction(
            &self,
            _tx: mp_rpc::v0_10_2::BroadcastedInvokeTxn,
        ) -> Result<mp_rpc::v0_9_0::AddInvokeTransactionResult, SubmitTransactionError> {
            Err(SubmitTransactionError::Unsupported)
        }
    }

    #[async_trait]
    impl TransactionLookup for FailingSubmitTransaction {
        async fn received_transaction(&self, _hash: Felt) -> Option<bool> {
            None
        }

        async fn subscribe_new_transactions(&self) -> Option<tokio::sync::broadcast::Receiver<Felt>> {
            None
        }

        async fn feeder_transaction_status(
            &self,
            _hash: Felt,
        ) -> Result<Option<ProviderTransactionStatus>, SubmitTransactionError> {
            Err(SubmitTransactionError::Internal(anyhow::anyhow!("upstream feeder failure")))
        }

        async fn feeder_transaction(
            &self,
            _hash: Felt,
        ) -> Result<Option<ProviderTransactionResponse>, SubmitTransactionError> {
            Err(SubmitTransactionError::Internal(anyhow::anyhow!("upstream feeder failure")))
        }
    }

    #[tokio::test]
    async fn get_transaction_status_propagates_submit_provider_errors() {
        let backend = backend_for_tests();
        let provider: Arc<dyn TransactionLookup> = Arc::new(FailingSubmitTransaction);

        let err = transaction_status_response(TX_HASH, &backend, &provider).await.unwrap_err();

        assert!(matches!(err, GatewayError::InternalServerError));
    }

    #[tokio::test]
    async fn get_transaction_propagates_submit_provider_errors() {
        let backend = backend_for_tests();
        let provider: Arc<dyn TransactionLookup> = Arc::new(FailingSubmitTransaction);

        let err = transaction_response(TX_HASH, &backend, &provider).await.unwrap_err();

        assert!(matches!(err, GatewayError::InternalServerError));
    }

    #[tokio::test]
    async fn get_transaction_returns_not_received_for_preconfirmed_backend_tx() {
        let backend = backend_for_tests();
        backend
            .write_access()
            .new_preconfirmed(mc_db::preconfirmed::PreconfirmedBlock::new_with_content(
                mp_block::header::PreconfirmedHeader { block_number: 0, ..Default::default() },
                [preconfirmed_tx()],
                std::iter::empty::<Arc<mp_transactions::validated::ValidatedTransaction>>(),
            ))
            .expect("Failed to persist preconfirmed block");
        let provider = StubSubmitTransaction::with_status(ProviderTransactionStatus::not_received());

        let response = transaction_response(TX_HASH, &backend, &provider).await.unwrap();

        assert_eq!(response, ProviderTransactionResponse::not_received());
    }

    #[tokio::test]
    async fn get_block_hash_by_id_and_block_id_by_hash_roundtrip() {
        let (backend, _, _) = submit_provider();
        backend
            .write_access()
            .add_full_block_with_classes(&empty_block(0), &[], true)
            .expect("Failed to persist confirmed block");
        backend
            .write_access()
            .add_full_block_with_classes(&empty_block(1), &[], true)
            .expect("Failed to persist confirmed block");

        let block_hash = block_hash_by_id_response(0, &backend).unwrap();
        let block_id = block_id_by_hash_response(block_hash, &backend).unwrap();
        let latest_block_hash = block_hash_by_id_response(1, &backend).unwrap();
        let latest_block_id = block_id_by_hash_response(latest_block_hash, &backend).unwrap();

        assert_eq!(block_id, 0);
        assert_eq!(latest_block_id, 1);
    }

    #[tokio::test]
    async fn get_block_hash_by_id_rejects_out_of_range_requests() {
        let (backend, _, _) = submit_provider();
        backend
            .write_access()
            .add_full_block_with_classes(&empty_block(0), &[], true)
            .expect("Failed to persist confirmed block");

        let err = block_hash_by_id_response(1, &backend).unwrap_err();

        assert!(matches!(
            err,
            GatewayError::StarknetError(StarknetError { code: StarknetErrorCode::MalformedRequest, .. })
        ));
    }

    #[tokio::test]
    async fn get_block_id_by_hash_rejects_unknown_hash() {
        let (backend, _, _) = submit_provider();
        backend
            .write_access()
            .add_full_block_with_classes(&empty_block(0), &[], true)
            .expect("Failed to persist confirmed block");

        let err = block_id_by_hash_response(Felt::ONE, &backend).unwrap_err();

        assert!(matches!(
            err,
            GatewayError::StarknetError(StarknetError { code: StarknetErrorCode::BlockNotFound, .. })
        ));
    }
}
