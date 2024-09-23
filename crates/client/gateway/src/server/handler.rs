use std::sync::Arc;

use hyper::{body, Body, Request, Response};
use mc_db::MadaraBackend;
use mc_rpc::providers::AddTransactionProvider;
use mp_block::{BlockId, BlockTag, MadaraBlock, MadaraPendingBlock};
use mp_class::ContractClass;
use mp_gateway::{
    block::{BlockProvider, BlockStatus, PendingBlockProvider},
    state_update::{PendingStateUpdateProvider, StateUpdateProvider},
};
use serde_json::json;
use starknet_core::types::{
    BroadcastedDeclareTransaction, BroadcastedDeployAccountTransaction, BroadcastedInvokeTransaction,
    BroadcastedTransaction,
};
use starknet_types_core::felt::Felt;

use crate::error::StarknetError;

use super::{
    error::{GatewayError, OptionExt, ResultExt},
    helpers::{
        block_id_from_params, create_json_response, create_response_with_json_body, get_params_from_request,
        include_block_params,
    },
};

pub async fn handle_get_block(req: Request<Body>, backend: Arc<MadaraBackend>) -> Result<Response<Body>, GatewayError> {
    let params = get_params_from_request(&req);
    let block_id = block_id_from_params(&params).or_internal_server_error("Retrieving block id")?;

    let block = backend
        .get_block(&block_id)
        .or_internal_server_error(format!("Retrieving block {block_id}"))?
        .ok_or(StarknetError::block_not_found())?;

    if let Ok(block) = MadaraBlock::try_from(block.clone()) {
        let last_l1_confirmed_block =
            backend.get_l1_last_confirmed_block().or_internal_server_error("Retrieving last l1 confirmed block")?;

        let status = if Some(block.info.header.block_number) <= last_l1_confirmed_block {
            BlockStatus::AcceptedOnL1
        } else {
            BlockStatus::AcceptedOnL2
        };

        let block_provider = BlockProvider::new(block, status);
        Ok(create_json_response(hyper::StatusCode::OK, &block_provider))
    } else if let Ok(block) = MadaraPendingBlock::try_from(block) {
        let block_provider = PendingBlockProvider::new(block);
        Ok(create_json_response(hyper::StatusCode::OK, &block_provider))
    } else {
        Err(GatewayError::InternalServerError.into())
    }
}

pub async fn handle_get_state_update(
    req: Request<Body>,
    backend: Arc<MadaraBackend>,
) -> Result<Response<Body>, GatewayError> {
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

            let state_update = PendingStateUpdateProvider { old_root, state_diff: state_diff.into() };

            let json_response = if let Some(block) = with_block {
                let block = MadaraPendingBlock::try_from(block.clone())
                    .or_internal_server_error("Attempting to convert pending block to non-pending")?;
                let block_provider = PendingBlockProvider::new(block);

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
                .ok_or_internal_server_error(format!("Converting potentially pending block to non-pending"))?;

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

            let state_update = StateUpdateProvider {
                block_hash: block_info.block_hash,
                old_root,
                new_root: block_info.header.global_state_root,
                state_diff: state_diff.into(),
            };

            let json_response = if let Some(block) = with_block {
                let block = MadaraBlock::try_from(block.clone())
                    .or_internal_server_error("Attempting to convert pending block to non-pending")?;
                let block_provider = BlockProvider::new(block, BlockStatus::AcceptedOnL2);

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

pub async fn handle_get_class_by_hash(
    req: Request<Body>,
    backend: Arc<MadaraBackend>,
) -> Result<Response<Body>, GatewayError> {
    let params = get_params_from_request(&req);
    let block_id = block_id_from_params(&params).unwrap_or(BlockId::Tag(BlockTag::Latest));

    let class_hash = params.get("classHash").ok_or(StarknetError::missing_class_hash())?;
    let class_hash = Felt::from_hex(class_hash).map_err(|e| StarknetError::invalid_class_hash(e))?;

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
            create_response_with_json_body(hyper::StatusCode::OK, &class)
        }
    };

    Ok(json_response)
}

pub async fn handle_add_transaction(
    req: Request<Body>,
    add_transaction_provider: Arc<dyn AddTransactionProvider>,
) -> Result<Response<Body>, GatewayError> {
    let whole_body = body::to_bytes(req.into_body()).await.or_internal_server_error("Failed to read request body")?;

    let transaction = serde_json::from_slice::<BroadcastedTransaction>(whole_body.as_ref())
        .map_err(|e| GatewayError::StarknetError(StarknetError::malformed_request(e)))?;

    let response = match transaction {
        BroadcastedTransaction::Declare(tx) => declare_transaction(tx, add_transaction_provider).await,
        BroadcastedTransaction::DeployAccount(tx) => deploy_account_transaction(tx, add_transaction_provider).await,
        BroadcastedTransaction::Invoke(tx) => invoke_transaction(tx, add_transaction_provider).await,
    };

    Ok(response)
}

async fn declare_transaction(
    tx: BroadcastedDeclareTransaction,
    add_transaction_provider: Arc<dyn AddTransactionProvider>,
) -> Response<Body> {
    match add_transaction_provider.add_declare_transaction(tx).await {
        Ok(result) => create_json_response(hyper::StatusCode::OK, &result),
        Err(e) => create_json_response(hyper::StatusCode::OK, &e),
    }
}

async fn deploy_account_transaction(
    tx: BroadcastedDeployAccountTransaction,
    add_transaction_provider: Arc<dyn AddTransactionProvider>,
) -> Response<Body> {
    match add_transaction_provider.add_deploy_account_transaction(tx).await {
        Ok(result) => create_json_response(hyper::StatusCode::OK, &result),
        Err(e) => create_json_response(hyper::StatusCode::OK, &e),
    }
}

async fn invoke_transaction(
    tx: BroadcastedInvokeTransaction,
    add_transaction_provider: Arc<dyn AddTransactionProvider>,
) -> Response<Body> {
    match add_transaction_provider.add_invoke_transaction(tx).await {
        Ok(result) => create_json_response(hyper::StatusCode::OK, &result),
        Err(e) => create_json_response(hyper::StatusCode::OK, &e),
    }
}