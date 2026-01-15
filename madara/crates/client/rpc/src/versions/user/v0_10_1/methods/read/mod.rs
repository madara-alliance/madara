use crate::versions::user::v0_10_0::StarknetReadRpcApiV0_10_0Server as V0_10_0Impl;
use crate::versions::user::v0_10_1::StarknetReadRpcApiV0_10_1Server;
use crate::versions::user::v0_8_1::StarknetReadRpcApiV0_8_1Server as V0_8_1Impl;
use crate::versions::user::v0_9_0::StarknetReadRpcApiV0_9_0Server as V0_9_0Impl;
use crate::{Starknet, StarknetRpcApiError};
use jsonrpsee::core::{async_trait, RpcResult};
use mp_chain_config::RpcVersion;
use mp_convert::Felt;
use mp_rpc::v0_10_1::{
    BlockHashAndNumber, BroadcastedTxn, ContractStorageKeysItem, EventFilterWithPageRequest, EventsChunk, FeeEstimate,
    FunctionCall, GetStorageProofResult, MaybeDeprecatedContractClass, MaybePreConfirmedBlockWithTxHashes,
    MaybePreConfirmedBlockWithTxs, MaybePreConfirmedStateUpdate, MessageFeeEstimate, MsgFromL1, ResponseFlag,
    SimulationFlagForEstimateFee, StarknetGetBlockWithTxsAndReceiptsResult, SyncingStatus,
    TxnFinalityAndExecutionStatus, TxnReceiptWithBlockInfo, TxnWithHash,
};

// v0.10.1 specific implementation
pub mod get_events;

// Re-use BlockId from v0.10.0
use mp_rpc::v0_10_0::BlockId;

#[async_trait]
impl StarknetReadRpcApiV0_10_1Server for Starknet {
    fn spec_version(&self) -> RpcResult<String> {
        Ok(RpcVersion::RPC_VERSION_0_10_1.to_string())
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
        V0_9_0Impl::call(self, request, block_id).await
    }

    fn get_block_transaction_count(&self, block_id: BlockId) -> RpcResult<u128> {
        V0_9_0Impl::get_block_transaction_count(self, block_id)
    }

    async fn estimate_fee(
        &self,
        request: Vec<BroadcastedTxn>,
        simulation_flags: Vec<SimulationFlagForEstimateFee>,
        block_id: BlockId,
    ) -> RpcResult<Vec<FeeEstimate>> {
        V0_9_0Impl::estimate_fee(self, request, simulation_flags, block_id).await
    }

    async fn estimate_message_fee(&self, message: MsgFromL1, block_id: BlockId) -> RpcResult<MessageFeeEstimate> {
        V0_10_0Impl::estimate_message_fee(self, message, block_id).await
    }

    fn get_block_with_receipts(&self, block_id: BlockId) -> RpcResult<StarknetGetBlockWithTxsAndReceiptsResult> {
        V0_10_0Impl::get_block_with_receipts(self, block_id)
    }

    fn get_block_with_tx_hashes(&self, block_id: BlockId) -> RpcResult<MaybePreConfirmedBlockWithTxHashes> {
        V0_10_0Impl::get_block_with_tx_hashes(self, block_id)
    }

    fn get_block_with_txs(
        &self,
        block_id: BlockId,
        _response_flags: Option<Vec<ResponseFlag>>,
    ) -> RpcResult<MaybePreConfirmedBlockWithTxs> {
        // Note: response_flags is accepted but not yet used.
        // INCLUDE_PROOF_FACTS would require fetching proof_facts from storage,
        // which is not yet implemented.
        V0_10_0Impl::get_block_with_txs(self, block_id)
    }

    fn get_class_at(&self, block_id: BlockId, contract_address: Felt) -> RpcResult<MaybeDeprecatedContractClass> {
        V0_9_0Impl::get_class_at(self, block_id, contract_address)
    }

    fn get_class_hash_at(&self, block_id: BlockId, contract_address: Felt) -> RpcResult<Felt> {
        V0_9_0Impl::get_class_hash_at(self, block_id, contract_address)
    }

    fn get_class(&self, block_id: BlockId, class_hash: Felt) -> RpcResult<MaybeDeprecatedContractClass> {
        V0_9_0Impl::get_class(self, block_id, class_hash)
    }

    fn get_events(&self, filter: EventFilterWithPageRequest) -> RpcResult<EventsChunk> {
        Ok(get_events::get_events(self, filter)?)
    }

    fn get_nonce(&self, block_id: BlockId, contract_address: Felt) -> RpcResult<Felt> {
        V0_9_0Impl::get_nonce(self, block_id, contract_address)
    }

    fn get_storage_at(&self, contract_address: Felt, key: Felt, block_id: BlockId) -> RpcResult<Felt> {
        V0_9_0Impl::get_storage_at(self, contract_address, key, block_id)
    }

    fn get_transaction_by_block_id_and_index(
        &self,
        block_id: BlockId,
        index: u64,
        _response_flags: Option<Vec<ResponseFlag>>,
    ) -> RpcResult<TxnWithHash> {
        // Note: response_flags is accepted but not yet used.
        // INCLUDE_PROOF_FACTS would require fetching proof_facts from storage,
        // which is not yet implemented.
        V0_9_0Impl::get_transaction_by_block_id_and_index(self, block_id, index)
    }

    fn get_transaction_by_hash(
        &self,
        transaction_hash: Felt,
        _response_flags: Option<Vec<ResponseFlag>>,
    ) -> RpcResult<TxnWithHash> {
        // Note: response_flags is accepted but not yet used.
        // INCLUDE_PROOF_FACTS would require fetching proof_facts from storage,
        // which is not yet implemented.
        V0_9_0Impl::get_transaction_by_hash(self, transaction_hash)
    }

    fn get_transaction_receipt(&self, transaction_hash: Felt) -> RpcResult<TxnReceiptWithBlockInfo> {
        V0_9_0Impl::get_transaction_receipt(self, transaction_hash)
    }

    async fn get_transaction_status(&self, transaction_hash: Felt) -> RpcResult<TxnFinalityAndExecutionStatus> {
        V0_9_0Impl::get_transaction_status(self, transaction_hash).await
    }

    fn get_state_update(&self, block_id: BlockId) -> RpcResult<MaybePreConfirmedStateUpdate> {
        V0_10_0Impl::get_state_update(self, block_id)
    }

    fn get_storage_proof(
        &self,
        block_id: BlockId,
        class_hashes: Option<Vec<Felt>>,
        contract_addresses: Option<Vec<Felt>>,
        contracts_storage_keys: Option<Vec<ContractStorageKeysItem>>,
    ) -> RpcResult<GetStorageProofResult> {
        let block_view = self.resolve_view_on(block_id)?;

        // Convert StorageKey to Felt for v0.8.1 compatibility
        let contracts_storage_keys_v0_8_1 = contracts_storage_keys.map(|keys| {
            keys.into_iter()
                .map(|item| mp_rpc::v0_8_1::ContractStorageKeysItem {
                    contract_address: item.contract_address,
                    storage_keys: item
                        .storage_keys
                        .into_iter()
                        .map(|key| Felt::from_hex(&key).unwrap_or(Felt::ZERO))
                        .collect(),
                })
                .collect()
        });

        V0_8_1Impl::get_storage_proof(
            self,
            mp_rpc::v0_8_1::BlockId::Number(
                block_view.latest_confirmed_block_n().ok_or(StarknetRpcApiError::NoBlocks)?,
            ),
            class_hashes,
            contract_addresses,
            contracts_storage_keys_v0_8_1,
        )
    }

    fn get_compiled_casm(&self, class_hash: Felt) -> RpcResult<serde_json::Value> {
        V0_8_1Impl::get_compiled_casm(self, class_hash)
    }
}
