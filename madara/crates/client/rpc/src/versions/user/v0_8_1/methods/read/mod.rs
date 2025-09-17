use crate::versions::user::v0_7_1::StarknetReadRpcApiV0_7_1Server as V0_7_1Impl;
use crate::versions::user::v0_8_1::StarknetReadRpcApiV0_8_1Server;
use crate::Starknet;
use jsonrpsee::core::{async_trait, RpcResult};
use mp_chain_config::RpcVersion;
use mp_rpc::v0_8_1::{
    BlockHashAndNumber, BlockId, BroadcastedTxn, ContractStorageKeysItem, EventFilterWithPageRequest, EventsChunk,
    FeeEstimate, FunctionCall, GetStorageProofResult, MaybeDeprecatedContractClass, MaybePendingBlockWithTxHashes,
    MaybePendingBlockWithTxs, MaybePendingStateUpdate, MsgFromL1, SimulationFlagForEstimateFee,
    StarknetGetBlockWithTxsAndReceiptsResult, SyncingStatus, TxnFinalityAndExecutionStatus, TxnReceiptWithBlockInfo,
    TxnWithHash,
};
use starknet_types_core::felt::Felt;

pub mod estimate_fee;
pub mod estimate_message_fee;
pub mod get_block_with_receipts;
pub mod get_block_with_tx_hashes;
pub mod get_block_with_txs;
pub mod get_compiled_casm;
pub mod get_storage_proof;
pub mod get_transaction_by_block_id_and_index;
pub mod get_transaction_by_hash;
pub mod get_transaction_receipt;

#[async_trait]
impl StarknetReadRpcApiV0_8_1Server for Starknet {
    fn spec_version(&self) -> RpcResult<String> {
        Ok(RpcVersion::RPC_VERSION_0_8_1.to_string())
    }

    fn block_number(&self) -> RpcResult<u64> {
        V0_7_1Impl::block_number(self)
    }

    fn block_hash_and_number(&self) -> RpcResult<BlockHashAndNumber> {
        V0_7_1Impl::block_hash_and_number(self)
    }

    fn chain_id(&self) -> RpcResult<Felt> {
        V0_7_1Impl::chain_id(self)
    }

    fn syncing(&self) -> RpcResult<SyncingStatus> {
        V0_7_1Impl::syncing(self)
    }

    async fn call(&self, request: FunctionCall, block_id: BlockId) -> RpcResult<Vec<Felt>> {
        V0_7_1Impl::call(self, request, block_id).await
    }

    fn get_block_transaction_count(&self, block_id: BlockId) -> RpcResult<u128> {
        V0_7_1Impl::get_block_transaction_count(self, block_id)
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
        V0_7_1Impl::get_class_at(self, block_id, contract_address)
    }

    fn get_class_hash_at(&self, block_id: BlockId, contract_address: Felt) -> RpcResult<Felt> {
        V0_7_1Impl::get_class_hash_at(self, block_id, contract_address)
    }

    fn get_class(&self, block_id: BlockId, class_hash: Felt) -> RpcResult<MaybeDeprecatedContractClass> {
        V0_7_1Impl::get_class(self, block_id, class_hash)
    }

    fn get_events(&self, filter: EventFilterWithPageRequest) -> RpcResult<EventsChunk> {
        V0_7_1Impl::get_events(self, filter)
    }

    fn get_nonce(&self, block_id: BlockId, contract_address: Felt) -> RpcResult<Felt> {
        V0_7_1Impl::get_nonce(self, block_id, contract_address)
    }

    fn get_storage_at(&self, contract_address: Felt, key: Felt, block_id: BlockId) -> RpcResult<Felt> {
        V0_7_1Impl::get_storage_at(self, contract_address, key, block_id)
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
        V0_7_1Impl::get_transaction_status(self, transaction_hash).await
    }

    fn get_state_update(&self, block_id: BlockId) -> RpcResult<MaybePendingStateUpdate> {
        V0_7_1Impl::get_state_update(self, block_id)
    }

    fn get_storage_proof(
        &self,
        block_id: BlockId,
        class_hashes: Option<Vec<Felt>>,
        contract_addresses: Option<Vec<Felt>>,
        contracts_storage_keys: Option<Vec<ContractStorageKeysItem>>,
    ) -> RpcResult<GetStorageProofResult> {
        Ok(get_storage_proof::get_storage_proof(
            self,
            block_id,
            class_hashes,
            contract_addresses,
            contracts_storage_keys,
        )?)
    }

    fn get_compiled_casm(&self, class_hash: Felt) -> RpcResult<serde_json::Value> {
        Ok(get_compiled_casm::get_compiled_casm(self, class_hash)?)
    }
}