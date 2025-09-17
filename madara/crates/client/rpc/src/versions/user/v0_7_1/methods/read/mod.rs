use crate::versions::user::v0_7_1::StarknetReadRpcApiV0_7_1Server;
use crate::{Starknet, StarknetRpcApiError};
use jsonrpsee::core::{async_trait, RpcResult};
use mp_chain_config::RpcVersion;
use mp_convert::{Felt, ToFelt};
use mp_rpc::v0_7_1::{
    BlockHashAndNumber, BlockId, BroadcastedTxn, EventFilterWithPageRequest, EventsChunk, FeeEstimate, FunctionCall,
    MaybeDeprecatedContractClass, MaybePendingBlockWithTxHashes, MaybePendingBlockWithTxs, MaybePendingStateUpdate,
    MsgFromL1, SimulationFlagForEstimateFee, StarknetGetBlockWithTxsAndReceiptsResult, SyncingStatus,
    TxnFinalityAndExecutionStatus, TxnReceiptWithBlockInfo, TxnWithHash,
};

mod block_hash_and_number;
mod call;
mod estimate_fee;
mod estimate_message_fee;
mod get_block_transaction_count;
mod get_block_with_receipts;
mod get_block_with_tx_hashes;
mod get_block_with_txs;
mod get_class;
mod get_class_at;
mod get_class_hash_at;
mod get_events;
mod get_nonce;
mod get_state_update;
mod get_storage_at;
mod get_transaction_by_block_id_and_index;
mod get_transaction_by_hash;
mod get_transaction_receipt;
mod get_transaction_status;
pub mod syncing;

#[async_trait]
impl StarknetReadRpcApiV0_7_1Server for Starknet {
    fn spec_version(&self) -> RpcResult<String> {
        Ok(RpcVersion::RPC_VERSION_0_7_1.to_string())
    }

    fn block_number(&self) -> RpcResult<u64> {
        self.backend.latest_confirmed_block_n().ok_or(StarknetRpcApiError::NoBlocks.into())
    }

    fn block_hash_and_number(&self) -> RpcResult<BlockHashAndNumber> {
        Ok(block_hash_and_number::block_hash_and_number(self)?)
    }

    fn chain_id(&self) -> RpcResult<Felt> {
        Ok(self.backend.chain_config().chain_id.to_felt())
    }

    fn syncing(&self) -> RpcResult<SyncingStatus> {
        Ok(syncing::syncing(self)?)
    }

    async fn call(&self, request: FunctionCall, block_id: BlockId) -> RpcResult<Vec<Felt>> {
        Ok(call::call(self, request, block_id).await?)
    }


    fn get_block_transaction_count(&self, block_id: BlockId) -> RpcResult<u128> {
        Ok(get_block_transaction_count::get_block_transaction_count(self, block_id)?)
    }

    async fn estimate_fee(
        &self,
        request: Vec<BroadcastedTxn>,
        simulation_flags: Vec<SimulationFlagForEstimateFee>,
        block_id: BlockId,
    ) -> RpcResult<Vec<FeeEstimate>> {
        Ok(estimate_fee::estimate_fee(self, request, simulation_flags, block_id).await?)
    }

    async fn estimate_message_fee(&self, message: MsgFromL1, block_id: BlockId) -> RpcResult<FeeEstimate> {
        Ok(estimate_message_fee::estimate_message_fee(self, message, block_id).await?)
    }

    fn get_block_with_receipts(&self, block_id: BlockId) -> RpcResult<StarknetGetBlockWithTxsAndReceiptsResult> {
        Ok(get_block_with_receipts::get_block_with_receipts(self, block_id)?)
    }

    fn get_block_with_tx_hashes(&self, block_id: BlockId) -> RpcResult<MaybePendingBlockWithTxHashes> {
        Ok(get_block_with_tx_hashes::get_block_with_tx_hashes(self, block_id)?)
    }

    fn get_block_with_txs(&self, block_id: BlockId) -> RpcResult<MaybePendingBlockWithTxs> {
        Ok(get_block_with_txs::get_block_with_txs(self, block_id)?)
    }

    fn get_class_at(&self, block_id: BlockId, contract_address: Felt) -> RpcResult<MaybeDeprecatedContractClass> {
        Ok(get_class_at::get_class_at(self, block_id, contract_address)?)
    }

    fn get_class_hash_at(&self, block_id: BlockId, contract_address: Felt) -> RpcResult<Felt> {
        Ok(get_class_hash_at::get_class_hash_at(self, block_id, contract_address)?)
    }

    fn get_class(&self, block_id: BlockId, class_hash: Felt) -> RpcResult<MaybeDeprecatedContractClass> {
        Ok(get_class::get_class(self, block_id, class_hash)?)
    }

    fn get_events(&self, filter: EventFilterWithPageRequest) -> RpcResult<EventsChunk> {
        Ok(get_events::get_events(self, filter)?)
    }

    fn get_nonce(&self, block_id: BlockId, contract_address: Felt) -> RpcResult<Felt> {
        Ok(get_nonce::get_nonce(self, block_id, contract_address)?)
    }

    fn get_storage_at(&self, contract_address: Felt, key: Felt, block_id: BlockId) -> RpcResult<Felt> {
        Ok(get_storage_at::get_storage_at(self, contract_address, key, block_id)?)
    }

    fn get_transaction_by_block_id_and_index(&self, block_id: BlockId, index: u64) -> RpcResult<TxnWithHash> {
        Ok(get_transaction_by_block_id_and_index::get_transaction_by_block_id_and_index(self, block_id, index)?)
    }

    fn get_transaction_by_hash(&self, transaction_hash: Felt) -> RpcResult<TxnWithHash> {
        Ok(get_transaction_by_hash::get_transaction_by_hash(self, transaction_hash)?)
    }

    fn get_transaction_receipt(&self, transaction_hash: Felt) -> RpcResult<TxnReceiptWithBlockInfo> {
        Ok(get_transaction_receipt::get_transaction_receipt(self, transaction_hash)?)
    }

    async fn get_transaction_status(&self, transaction_hash: Felt) -> RpcResult<TxnFinalityAndExecutionStatus> {
        Ok(get_transaction_status::get_transaction_status(self, transaction_hash).await?)
    }

    fn get_state_update(&self, block_id: BlockId) -> RpcResult<MaybePendingStateUpdate> {
        Ok(get_state_update::get_state_update(self, block_id)?)
    }
}
