use super::common::response_flags_include_proof_facts;
use crate::errors::{StarknetRpcApiError, StarknetRpcResult};
use crate::Starknet;
use mp_rpc::v0_10_2::{ResponseFlag, TxnWithHashAndProofFacts};
use starknet_types_core::felt::Felt;

pub fn get_transaction_by_hash(
    starknet: &Starknet,
    transaction_hash: Felt,
    response_flags: Option<Vec<ResponseFlag>>,
) -> StarknetRpcResult<TxnWithHashAndProofFacts> {
    let include_proof_facts = response_flags_include_proof_facts(response_flags);
    let view = starknet.backend.view_on_latest();
    let res = view.find_transaction_by_hash(&transaction_hash)?.ok_or(StarknetRpcApiError::TxnHashNotFound)?;
    let tx = res.get_transaction()?;

    Ok(TxnWithHashAndProofFacts { transaction: tx.transaction.to_rpc_v0_10_2(include_proof_facts), transaction_hash })
}
