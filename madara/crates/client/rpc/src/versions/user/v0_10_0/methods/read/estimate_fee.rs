use crate::errors::StarknetRpcApiError;
use crate::errors::StarknetRpcResult;
use crate::utils::tx_api_to_blockifier;
use crate::Starknet;
use blockifier::transaction::account_transaction::ExecutionFlags;
use mc_exec::execution::TxInfo;
use mc_exec::MadaraBlockViewExecutionExt;
use mc_exec::EXECUTION_UNSUPPORTED_BELOW_VERSION;
use mp_convert::ToFelt;
use mp_rpc::v0_10_0::{BlockId, BroadcastedTxn, FeeEstimate, SimulationFlagForEstimateFee};
use mp_transactions::{IntoStarknetApiExt, ToBlockifierError};
use starknet_types_core::felt::Felt;

/// Extract contract address from a broadcasted transaction for validation
fn get_contract_address_from_tx(tx: &BroadcastedTxn) -> Option<Felt> {
    match tx {
        BroadcastedTxn::Invoke(invoke_tx) => match invoke_tx {
            mp_rpc::v0_10_0::BroadcastedInvokeTxn::V0(tx) => Some(tx.contract_address),
            mp_rpc::v0_10_0::BroadcastedInvokeTxn::V1(tx) => Some(tx.sender_address),
            mp_rpc::v0_10_0::BroadcastedInvokeTxn::V3(tx) => Some(tx.sender_address),
            mp_rpc::v0_10_0::BroadcastedInvokeTxn::QueryV0(tx) => Some(tx.contract_address),
            mp_rpc::v0_10_0::BroadcastedInvokeTxn::QueryV1(tx) => Some(tx.sender_address),
            mp_rpc::v0_10_0::BroadcastedInvokeTxn::QueryV3(tx) => Some(tx.sender_address),
        },
        BroadcastedTxn::Declare(declare_tx) => match declare_tx {
            mp_rpc::v0_10_0::BroadcastedDeclareTxn::V1(tx) => Some(tx.sender_address),
            mp_rpc::v0_10_0::BroadcastedDeclareTxn::V2(tx) => Some(tx.sender_address),
            mp_rpc::v0_10_0::BroadcastedDeclareTxn::V3(tx) => Some(tx.sender_address),
            mp_rpc::v0_10_0::BroadcastedDeclareTxn::QueryV1(tx) => Some(tx.sender_address),
            mp_rpc::v0_10_0::BroadcastedDeclareTxn::QueryV2(tx) => Some(tx.sender_address),
            mp_rpc::v0_10_0::BroadcastedDeclareTxn::QueryV3(tx) => Some(tx.sender_address),
        },
        // DeployAccount transactions are deploying new contracts, so we don't check for existence
        BroadcastedTxn::DeployAccount(_) => None,
    }
}

/// Estimate the fee associated with transaction
///
/// # Arguments
///
/// * `request` - starknet transaction request
/// * `block_id` - hash of the requested block, number (height), or tag
///
/// # Returns
///
/// * `fee_estimate` - fee estimate in gwei
pub async fn estimate_fee(
    starknet: &Starknet,
    request: Vec<BroadcastedTxn>,
    simulation_flags: Vec<SimulationFlagForEstimateFee>,
    block_id: BlockId,
) -> StarknetRpcResult<Vec<FeeEstimate>> {
    tracing::debug!("estimate fee on block_id {block_id:?}");
    let view = starknet.resolve_block_view(block_id)?;
    let mut exec_context = view.new_execution_context()?;

    if exec_context.protocol_version < EXECUTION_UNSUPPORTED_BELOW_VERSION {
        return Err(StarknetRpcApiError::unsupported_txn_version());
    }

    let validate = !simulation_flags.contains(&SimulationFlagForEstimateFee::SkipValidate);

    // RPC 0.10.0: Check contract existence for transactions
    let state_view = view.state_view();
    for tx in &request {
        if let Some(contract_address) = get_contract_address_from_tx(tx) {
            if state_view.get_contract_class_hash(&contract_address)?.is_none() {
                return Err(StarknetRpcApiError::ContractNotFound {
                    error: format!("Contract at address {:#x} not found", contract_address).into(),
                });
            }
        }
    }

    let transactions = request
        .into_iter()
        .map(|tx| {
            let only_query = tx.is_query();
            let (api_tx, _) =
                tx.into_starknet_api(view.backend().chain_config().chain_id.to_felt(), exec_context.protocol_version)?;
            let execution_flags = ExecutionFlags { only_query, charge_fee: false, validate, strict_nonce_check: true };
            Ok(tx_api_to_blockifier(api_tx, execution_flags)?)
        })
        .collect::<Result<Vec<_>, ToBlockifierError>>()?;

    let tips = transactions.iter().map(|tx| tx.tip().unwrap_or_default()).collect::<Vec<_>>();

    // spawn_blocking: avoid starving the tokio workers during execution.
    let (execution_results, exec_context) = mp_utils::spawn_blocking(move || {
        Ok::<_, mc_exec::Error>((exec_context.execute_transactions([], transactions)?, exec_context))
    })
    .await?;

    let fee_estimates = execution_results
        .iter()
        .zip(tips)
        .enumerate()
        .map(|(index, (result, tip))| {
            if result.execution_info.is_reverted() {
                return Err(StarknetRpcApiError::TxnExecutionError {
                    tx_index: index,
                    error: result.execution_info.revert_error.as_ref().map(|e| e.to_string()).unwrap_or_default(),
                });
            }
            Ok(FeeEstimate {
                common: exec_context.execution_result_to_fee_estimate_v0_9(result, tip)?,
                unit: mp_rpc::v0_10_0::PriceUnitFri::Fri,
            })
        })
        .collect::<Result<_, _>>()?;

    Ok(fee_estimates)
}
