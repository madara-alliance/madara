use crate::versions::user::v0_10_2::StarknetReadRpcApiV0_10_2Server as V0_10_2Impl;
use crate::versions::user::v0_10_3::StarknetReadRpcApiV0_10_3Server;
use crate::Starknet;
use jsonrpsee::core::{async_trait, RpcResult};
use mp_chain_config::RpcVersion;
use mp_convert::Felt;
use mp_rpc::v0_10_0::BlockId;
use mp_rpc::v0_10_3::{
    BlockHashAndNumber, BroadcastedTxn, ContractStorageKeysItem, EventFilterWithPageRequest, EventsChunk, FeeEstimate,
    FunctionCall, GetStorageAtResult, GetStorageProofResult, L1TxnHash, MaybeDeprecatedContractClass,
    MaybePreConfirmedBlockWithTxHashes, MaybePreConfirmedBlockWithTxsAndProofFacts, MaybePreConfirmedStateUpdate,
    MessageFeeEstimate, MessageStatus, MsgFromL1, ResponseFlag, SimulationFlagForEstimateFee,
    StarknetGetBlockWithTxsAndReceiptsResult, StorageResponseFlag, SyncingStatus, TxnFinalityAndExecutionStatus,
    TxnReceiptWithBlockInfo, TxnWithHashAndProofFacts,
};

// v0.10.3 has no semantic changes to the read API: everything except the spec
// version string delegates to the v0.10.2 implementation.
#[async_trait]
impl StarknetReadRpcApiV0_10_3Server for Starknet {
    fn spec_version(&self) -> RpcResult<String> {
        Ok(RpcVersion::RPC_VERSION_0_10_3.to_string())
    }

    fn block_number(&self) -> RpcResult<u64> {
        V0_10_2Impl::block_number(self)
    }

    fn block_hash_and_number(&self) -> RpcResult<BlockHashAndNumber> {
        V0_10_2Impl::block_hash_and_number(self)
    }

    fn chain_id(&self) -> RpcResult<Felt> {
        V0_10_2Impl::chain_id(self)
    }

    fn syncing(&self) -> RpcResult<SyncingStatus> {
        V0_10_2Impl::syncing(self)
    }

    async fn call(&self, request: FunctionCall, block_id: BlockId) -> RpcResult<Vec<Felt>> {
        V0_10_2Impl::call(self, request, block_id).await
    }

    fn get_block_transaction_count(&self, block_id: BlockId) -> RpcResult<u128> {
        V0_10_2Impl::get_block_transaction_count(self, block_id)
    }

    async fn estimate_fee(
        &self,
        request: Vec<BroadcastedTxn>,
        simulation_flags: Vec<SimulationFlagForEstimateFee>,
        block_id: BlockId,
    ) -> RpcResult<Vec<FeeEstimate>> {
        V0_10_2Impl::estimate_fee(self, request, simulation_flags, block_id).await
    }

    async fn estimate_message_fee(&self, message: MsgFromL1, block_id: BlockId) -> RpcResult<MessageFeeEstimate> {
        V0_10_2Impl::estimate_message_fee(self, message, block_id).await
    }

    fn get_block_with_receipts(
        &self,
        block_id: BlockId,
        response_flags: Option<Vec<ResponseFlag>>,
    ) -> RpcResult<StarknetGetBlockWithTxsAndReceiptsResult> {
        V0_10_2Impl::get_block_with_receipts(self, block_id, response_flags)
    }

    fn get_block_with_tx_hashes(&self, block_id: BlockId) -> RpcResult<MaybePreConfirmedBlockWithTxHashes> {
        V0_10_2Impl::get_block_with_tx_hashes(self, block_id)
    }

    fn get_block_with_txs(
        &self,
        block_id: BlockId,
        response_flags: Option<Vec<ResponseFlag>>,
    ) -> RpcResult<MaybePreConfirmedBlockWithTxsAndProofFacts> {
        V0_10_2Impl::get_block_with_txs(self, block_id, response_flags)
    }

    fn get_class_at(&self, block_id: BlockId, contract_address: Felt) -> RpcResult<MaybeDeprecatedContractClass> {
        V0_10_2Impl::get_class_at(self, block_id, contract_address)
    }

    fn get_class_hash_at(&self, block_id: BlockId, contract_address: Felt) -> RpcResult<Felt> {
        V0_10_2Impl::get_class_hash_at(self, block_id, contract_address)
    }

    fn get_class(&self, block_id: BlockId, class_hash: Felt) -> RpcResult<MaybeDeprecatedContractClass> {
        V0_10_2Impl::get_class(self, block_id, class_hash)
    }

    fn get_events(&self, filter: EventFilterWithPageRequest) -> RpcResult<EventsChunk> {
        V0_10_2Impl::get_events(self, filter)
    }

    fn get_nonce(&self, block_id: BlockId, contract_address: Felt) -> RpcResult<Felt> {
        V0_10_2Impl::get_nonce(self, block_id, contract_address)
    }

    fn get_storage_at(
        &self,
        contract_address: Felt,
        key: Felt,
        block_id: BlockId,
        response_flags: Option<Vec<StorageResponseFlag>>,
    ) -> RpcResult<GetStorageAtResult> {
        V0_10_2Impl::get_storage_at(self, contract_address, key, block_id, response_flags)
    }

    fn get_transaction_by_block_id_and_index(
        &self,
        block_id: BlockId,
        index: u64,
        response_flags: Option<Vec<ResponseFlag>>,
    ) -> RpcResult<TxnWithHashAndProofFacts> {
        V0_10_2Impl::get_transaction_by_block_id_and_index(self, block_id, index, response_flags)
    }

    fn get_transaction_by_hash(
        &self,
        transaction_hash: Felt,
        response_flags: Option<Vec<ResponseFlag>>,
    ) -> RpcResult<TxnWithHashAndProofFacts> {
        V0_10_2Impl::get_transaction_by_hash(self, transaction_hash, response_flags)
    }

    fn get_transaction_receipt(&self, transaction_hash: Felt) -> RpcResult<TxnReceiptWithBlockInfo> {
        V0_10_2Impl::get_transaction_receipt(self, transaction_hash)
    }

    async fn get_transaction_status(&self, transaction_hash: Felt) -> RpcResult<TxnFinalityAndExecutionStatus> {
        V0_10_2Impl::get_transaction_status(self, transaction_hash).await
    }

    fn get_state_update(
        &self,
        block_id: BlockId,
        contract_addresses: Option<Vec<Felt>>,
    ) -> RpcResult<MaybePreConfirmedStateUpdate> {
        V0_10_2Impl::get_state_update(self, block_id, contract_addresses)
    }

    fn get_messages_status(&self, transaction_hash: L1TxnHash) -> RpcResult<Vec<MessageStatus>> {
        V0_10_2Impl::get_messages_status(self, transaction_hash)
    }

    fn get_storage_proof(
        &self,
        block_id: BlockId,
        class_hashes: Option<Vec<Felt>>,
        contract_addresses: Option<Vec<Felt>>,
        contracts_storage_keys: Option<Vec<ContractStorageKeysItem>>,
    ) -> RpcResult<GetStorageProofResult> {
        V0_10_2Impl::get_storage_proof(self, block_id, class_hashes, contract_addresses, contracts_storage_keys)
    }

    fn get_compiled_casm(&self, class_hash: Felt) -> RpcResult<serde_json::Value> {
        V0_10_2Impl::get_compiled_casm(self, class_hash)
    }
}
