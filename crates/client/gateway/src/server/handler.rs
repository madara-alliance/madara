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

use crate::error::{StarknetError, StarknetErrorCode};

use super::{
    error::{GatewayError, ResultExt},
    helpers::{
        block_id_from_params, create_json_response, create_response_with_json_body, get_params_from_request,
        include_block_params,
    },
};

pub async fn handle_get_block(req: Request<Body>, backend: Arc<MadaraBackend>) -> Response<Body> {
    let params = get_params_from_request(&req);
    let block_id = match block_id_from_params(&params) {
        Ok(block_id) => block_id,
        // Return the error response if the request is malformed
        Err(e) => return e.into(),
    };

    let block = match backend
        .get_block(&block_id)
        .or_internal_server_error("get_block")
        .and_then(|block_id| block_id.ok_or(StarknetError::block_not_found().into()))
    {
        Ok(block) => block,
        Err(e) => return e.into(),
    };

    if let Ok(block) = MadaraBlock::try_from(block.clone()) {
        let last_l1_confirmed_block =
            match backend.get_l1_last_confirmed_block().or_internal_server_error("get_l1_last_confirmed_block") {
                Ok(block) => block,
                Err(e) => return e.into(),
            };

        let status = if Some(block.info.header.block_number) <= last_l1_confirmed_block {
            BlockStatus::AcceptedOnL1
        } else {
            BlockStatus::AcceptedOnL2
        };

        let block_provider = BlockProvider::new(block, status);
        create_json_response(hyper::StatusCode::OK, &block_provider)
    } else if let Ok(block) = MadaraPendingBlock::try_from(block) {
        let block_provider = PendingBlockProvider::new(block);
        create_json_response(hyper::StatusCode::OK, &block_provider)
    } else {
        GatewayError::InternalServerError.into()
    }
}

pub async fn handle_get_state_update(req: Request<Body>, backend: Arc<MadaraBackend>) -> Response<Body> {
    let params = get_params_from_request(&req);
    let block_id = match block_id_from_params(&params) {
        Ok(block_id) => block_id,
        // Return the error response if the request is malformed
        Err(e) => return e.into(),
    };

    let resolved_block_id = match backend
        .resolve_block_id(&block_id)
        .or_internal_server_error("resolve_block_id")
        .and_then(|block_id| block_id.ok_or(StarknetError::block_not_found().into()))
    {
        Ok(block_id) => block_id,
        Err(e) => return e.into(),
    };

    let state_diff = match backend
        .get_block_state_diff(&resolved_block_id)
        .or_internal_server_error("get_block_state_diff")
        .and_then(|block_id| block_id.ok_or(StarknetError::block_not_found().into()))
    {
        Ok(state_diff) => state_diff,
        Err(e) => return e.into(),
    };

    let with_block = if include_block_params(&params) {
        let block = match backend
            .get_block(&block_id)
            .or_internal_server_error("get_block")
            .and_then(|block_id| block_id.ok_or(StarknetError::block_not_found().into()))
        {
            Ok(block) => block,
            Err(e) => return e.into(),
        };
        Some(block)
    } else {
        None
    };

    match resolved_block_id.is_pending() {
        true => {
            let old_root = match backend
                .get_block_info(&BlockId::Tag(BlockTag::Latest))
                .or_internal_server_error("get_block_state_diff")
            {
                Ok(Some(block)) => match block.as_nonpending() {
                    Some(block) => block.header.global_state_root,
                    None => return GatewayError::InternalServerError.into(),
                },
                Ok(None) => Felt::ZERO, // The pending block is actually genesis, so old root is zero
                Err(e) => return e.into(),
            };
            let state_update = PendingStateUpdateProvider { old_root, state_diff: state_diff.into() };

            if let Some(block) = with_block {
                let block = match MadaraPendingBlock::try_from(block.clone()) {
                    Ok(block) => block,
                    Err(_) => return GatewayError::InternalServerError.into(),
                };
                let block_provider = PendingBlockProvider::new(block);
                create_json_response(
                    hyper::StatusCode::OK,
                    &json!({"block": block_provider, "state_update": state_update}),
                )
            } else {
                create_json_response(hyper::StatusCode::OK, &state_update)
            }
        }
        false => {
            let block_info = match backend
                .get_block_info(&resolved_block_id)
                .or_internal_server_error("get_block_info")
                .and_then(|block_id| block_id.ok_or(StarknetError::block_not_found().into()))
            {
                Ok(block_info) => block_info,
                Err(e) => return e.into(),
            };

            let block_info = match block_info.as_nonpending() {
                Some(block_info) => block_info,
                None => return GatewayError::InternalServerError.into(),
            };

            let old_root = if let Some(val) = block_info.header.block_number.checked_sub(1) {
                match backend.get_block_info(&BlockId::Number(val)).or_internal_server_error("get_block_info") {
                    Ok(Some(block)) => match block.as_nonpending() {
                        Some(block) => block.header.global_state_root,
                        None => return GatewayError::InternalServerError.into(),
                    },
                    Ok(None) => Felt::ZERO, // The pending block is actually genesis, so old root is zero
                    Err(e) => return e.into(),
                }
            } else {
                Felt::ZERO
            };

            let state_update = StateUpdateProvider {
                block_hash: block_info.block_hash,
                old_root,
                new_root: block_info.header.global_state_root,
                state_diff: state_diff.into(),
            };

            if let Some(block) = with_block {
                let block = match MadaraBlock::try_from(block.clone()) {
                    Ok(block) => block,
                    Err(_) => return GatewayError::InternalServerError.into(),
                };
                let block_provider = BlockProvider::new(block, BlockStatus::AcceptedOnL2);
                create_json_response(
                    hyper::StatusCode::OK,
                    &json!({"block": block_provider, "state_update": state_update}),
                )
            } else {
                create_json_response(hyper::StatusCode::OK, &state_update)
            }
        }
    }
}

pub async fn handle_get_class_by_hash(req: Request<Body>, backend: Arc<MadaraBackend>) -> Response<Body> {
    let params = get_params_from_request(&req);
    let block_id = block_id_from_params(&params).unwrap_or(BlockId::Tag(BlockTag::Latest));

    let class_hash = match params.get("classHash") {
        Some(class_hash) => class_hash,
        None => {
            return StarknetError {
                code: StarknetErrorCode::MalformedRequest,
                message: "Missing class_hash parameter".to_string(),
            }
            .into()
        }
    };

    let class_hash = match Felt::from_hex(class_hash) {
        Ok(class_hash) => class_hash,
        Err(e) => {
            return StarknetError {
                code: StarknetErrorCode::MalformedRequest,
                message: format!("Invalid class_hash: {}", e),
            }
            .into()
        }
    };

    let class_info = match backend
        .get_class_info(&block_id, &class_hash)
        .or_internal_server_error("get_class_by_hash")
        .and_then(|class| {
            class.ok_or(
                StarknetError {
                    code: StarknetErrorCode::UndeclaredClass,
                    message: format!("Class with hash {:#x} not found", class_hash),
                }
                .into(),
            )
        }) {
        Ok(class) => class,
        Err(e) => return e.into(),
    };

    match class_info.contract_class() {
        ContractClass::Sierra(flattened_sierra_class) => {
            create_json_response(hyper::StatusCode::OK, flattened_sierra_class.as_ref())
        }
        ContractClass::Legacy(compressed_legacy_contract_class) => {
            let class = match compressed_legacy_contract_class.as_ref().serialize_to_json() {
                Ok(class) => class,
                Err(e) => {
                    log::error!("Failed to serialize legacy class: {}", e);
                    return GatewayError::InternalServerError.into();
                }
            };
            create_response_with_json_body(hyper::StatusCode::OK, &class)
        }
    }
}

pub async fn handle_add_transaction(
    req: Request<Body>,
    add_transaction_provider: Arc<dyn AddTransactionProvider>,
) -> Response<Body> {
    let whole_body = match body::to_bytes(req.into_body()).await {
        Ok(body) => body,
        Err(e) => {
            log::error!("Failed to read request body: {}", e);
            return GatewayError::InternalServerError.into();
        }
    };

    let transaction = match serde_json::from_slice::<BroadcastedTransaction>(whole_body.as_ref()) {
        Ok(transaction) => transaction,
        Err(e) => {
            return GatewayError::StarknetError(StarknetError {
                code: StarknetErrorCode::MalformedRequest,
                message: format!("Failed to parse transaction: {}", e),
            })
            .into()
        }
    };

    match transaction {
        BroadcastedTransaction::Declare(tx) => declare_transaction(tx, add_transaction_provider).await,
        BroadcastedTransaction::DeployAccount(tx) => deploy_account_transaction(tx, add_transaction_provider).await,
        BroadcastedTransaction::Invoke(tx) => invoke_transaction(tx, add_transaction_provider).await,
    }
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
