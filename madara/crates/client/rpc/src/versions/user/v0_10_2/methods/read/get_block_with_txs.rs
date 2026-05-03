use super::common::response_flags_include_proof_facts;
use crate::errors::StarknetRpcResult;
use crate::Starknet;
use mp_block::MadaraMaybePreconfirmedBlockInfo;
use mp_rpc::v0_10_0::BlockId;
use mp_rpc::v0_10_2::{
    BlockStatus, BlockWithTxsAndProofFacts, MaybePreConfirmedBlockWithTxsAndProofFacts,
    PreConfirmedBlockWithTxsAndProofFacts, ResponseFlag, TxnWithHashAndProofFacts,
};

pub fn get_block_with_txs(
    starknet: &Starknet,
    block_id: BlockId,
    response_flags: Option<Vec<ResponseFlag>>,
) -> StarknetRpcResult<MaybePreConfirmedBlockWithTxsAndProofFacts> {
    let include_proof_facts = response_flags_include_proof_facts(response_flags);
    let view = starknet.resolve_block_view(block_id)?;
    let block_info = view.get_block_info()?;
    let txs = view
        .get_executed_transactions(..)?
        .into_iter()
        .map(|tx| TxnWithHashAndProofFacts {
            transaction: tx.transaction.to_rpc_v0_10_2(include_proof_facts),
            transaction_hash: *tx.receipt.transaction_hash(),
        })
        .collect();

    let status = if view.is_preconfirmed() {
        BlockStatus::PreConfirmed
    } else if view.is_on_l1() {
        BlockStatus::AcceptedOnL1
    } else {
        BlockStatus::AcceptedOnL2
    };

    match block_info {
        MadaraMaybePreconfirmedBlockInfo::Preconfirmed(block) => {
            Ok(MaybePreConfirmedBlockWithTxsAndProofFacts::PreConfirmed(PreConfirmedBlockWithTxsAndProofFacts {
                transactions: txs,
                pre_confirmed_block_header: block.header.to_rpc_v0_9(),
            }))
        }
        MadaraMaybePreconfirmedBlockInfo::Confirmed(block) => {
            Ok(MaybePreConfirmedBlockWithTxsAndProofFacts::Block(BlockWithTxsAndProofFacts {
                transactions: txs,
                status,
                block_header: block.to_rpc_v0_10(),
            }))
        }
    }
}
