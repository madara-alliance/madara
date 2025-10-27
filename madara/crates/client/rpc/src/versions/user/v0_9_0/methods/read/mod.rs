use crate::versions::user::v0_8_1::StarknetReadRpcApiV0_8_1Server as V0_8_1Impl;
use crate::versions::user::v0_9_0::StarknetReadRpcApiV0_9_0Server;
use crate::{Starknet, StarknetRpcApiError};
use jsonrpsee::core::{async_trait, RpcResult};
use mp_chain_config::RpcVersion;
use mp_convert::Felt;
use mp_rpc::v0_9_0::{
    BlockHashAndNumber, BlockId, BroadcastedTxn, ContractStorageKeysItem, EventFilterWithPageRequest, EventsChunk,
    FeeEstimate, FunctionCall, GetStorageProofResult, MaybeDeprecatedContractClass, MaybePreConfirmedBlockWithTxHashes,
    MaybePreConfirmedBlockWithTxs, MaybePreConfirmedStateUpdate, MessageFeeEstimate, MsgFromL1,
    SimulationFlagForEstimateFee, StarknetGetBlockWithTxsAndReceiptsResult, SyncingStatus,
    TxnFinalityAndExecutionStatus, TxnReceiptWithBlockInfo, TxnWithHash,
};

pub mod call;
pub mod estimate_fee;
pub mod estimate_message_fee;
pub mod get_block_transaction_count;
pub mod get_block_with_receipts;
pub mod get_block_with_tx_hashes;
pub mod get_block_with_txs;
pub mod get_class;
pub mod get_class_at;
pub mod get_class_hash_at;
pub mod get_events;
pub mod get_nonce;
pub mod get_state_update;
pub mod get_storage_at;
pub mod get_transaction_by_block_id_and_index;
pub mod get_transaction_by_hash;
pub mod get_transaction_receipt;
pub mod get_transaction_status;

#[async_trait]
impl StarknetReadRpcApiV0_9_0Server for Starknet {
    fn spec_version(&self) -> RpcResult<String> {
        Ok(RpcVersion::RPC_VERSION_0_9_0.to_string())
    }

    fn block_number(&self) -> RpcResult<u64> {
        V0_8_1Impl::block_number(self)
    }

    fn block_hash_and_number(&self) -> RpcResult<BlockHashAndNumber> {
        V0_8_1Impl::block_hash_and_number(self)
    }

    fn chain_id(&self) -> RpcResult<Felt> {
        V0_8_1Impl::chain_id(self)
    }

    fn syncing(&self) -> RpcResult<SyncingStatus> {
        V0_8_1Impl::syncing(self)
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

    async fn estimate_message_fee(&self, message: MsgFromL1, block_id: BlockId) -> RpcResult<MessageFeeEstimate> {
        Ok(estimate_message_fee::estimate_message_fee(self, message, block_id).await?)
    }

    fn get_block_with_receipts(&self, block_id: BlockId) -> RpcResult<StarknetGetBlockWithTxsAndReceiptsResult> {
        Ok(get_block_with_receipts::get_block_with_receipts(self, block_id)?)
    }

    fn get_block_with_tx_hashes(&self, block_id: BlockId) -> RpcResult<MaybePreConfirmedBlockWithTxHashes> {
        Ok(get_block_with_tx_hashes::get_block_with_tx_hashes(self, block_id)?)
    }

    fn get_block_with_txs(&self, block_id: BlockId) -> RpcResult<MaybePreConfirmedBlockWithTxs> {
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

    fn get_state_update(&self, block_id: BlockId) -> RpcResult<MaybePreConfirmedStateUpdate> {
        Ok(get_state_update::get_state_update(self, block_id)?)
    }
    fn get_storage_proof(
        &self,
        block_id: BlockId,
        class_hashes: Option<Vec<Felt>>,
        contract_addresses: Option<Vec<Felt>>,
        contracts_storage_keys: Option<Vec<ContractStorageKeysItem>>,
    ) -> RpcResult<GetStorageProofResult> {
        // support the new block id transparently (preconfirmed blocks are not allowed).
        let block_view = self.resolve_view_on(block_id)?;

        V0_8_1Impl::get_storage_proof(
            self,
            mp_rpc::v0_8_1::BlockId::Number(
                block_view.latest_confirmed_block_n().ok_or(StarknetRpcApiError::NoBlocks)?,
            ),
            class_hashes,
            contract_addresses,
            contracts_storage_keys,
        )
    }

    fn get_compiled_casm(&self, class_hash: Felt) -> RpcResult<serde_json::Value> {
        V0_8_1Impl::get_compiled_casm(self, class_hash)
    }
}
