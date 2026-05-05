use super::common::response_flags_include_proof_facts;
use crate::errors::{StarknetRpcApiError, StarknetRpcResult};
use crate::Starknet;
use mp_rpc::v0_10_0::BlockId;
use mp_rpc::v0_10_2::{ResponseFlag, TxnWithHashAndProofFacts};

pub fn get_transaction_by_block_id_and_index(
    starknet: &Starknet,
    block_id: BlockId,
    index: u64,
    response_flags: Option<Vec<ResponseFlag>>,
) -> StarknetRpcResult<TxnWithHashAndProofFacts> {
    let include_proof_facts = response_flags_include_proof_facts(response_flags);
    let view = starknet.resolve_block_view(block_id)?;
    let tx = view.get_executed_transaction(index)?.ok_or(StarknetRpcApiError::InvalidTxnIndex)?;

    Ok(TxnWithHashAndProofFacts {
        transaction: tx.transaction.to_rpc_v0_10_2(include_proof_facts),
        transaction_hash: *tx.receipt.transaction_hash(),
    })
}
