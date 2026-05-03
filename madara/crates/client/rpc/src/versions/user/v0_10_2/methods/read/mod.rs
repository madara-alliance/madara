use crate::versions::user::v0_10_0::StarknetReadRpcApiV0_10_0Server as V0_10_0Impl;
use crate::versions::user::v0_10_2::StarknetReadRpcApiV0_10_2Server;
use crate::versions::user::v0_8_1::StarknetReadRpcApiV0_8_1Server as V0_8_1Impl;
use crate::versions::user::v0_9_0::StarknetReadRpcApiV0_9_0Server as V0_9_0Impl;
use crate::Starknet;
use jsonrpsee::core::{async_trait, RpcResult};
use mp_chain_config::RpcVersion;
use mp_convert::Felt;
use mp_rpc::v0_10_2::{
    BlockHashAndNumber, BroadcastedTxn, ContractStorageKeysItem, EventFilterWithPageRequest, EventsChunk, FeeEstimate,
    FunctionCall, GetStorageAtResult, GetStorageProofResult, L1TxnHash, MaybeDeprecatedContractClass,
    MaybePreConfirmedBlockWithTxHashes, MaybePreConfirmedBlockWithTxsAndProofFacts, MaybePreConfirmedStateUpdate,
    MessageFeeEstimate, MessageStatus, MsgFromL1, ResponseFlag, SimulationFlagForEstimateFee,
    StarknetGetBlockWithTxsAndReceiptsResult, StorageResponseFlag, SyncingStatus, TxnFinalityAndExecutionStatus,
    TxnReceiptWithBlockInfo, TxnWithHashAndProofFacts,
};

// v0.10.2 specific implementation
mod common;
pub mod estimate_fee;
pub mod get_block_with_receipts;
pub mod get_block_with_txs;
pub mod get_events;
pub mod get_state_update;
pub mod get_storage_at;
pub mod get_storage_proof;
pub mod get_transaction_by_block_id_and_index;
pub mod get_transaction_by_hash;

// Re-use BlockId from v0.10.0
use mp_rpc::v0_10_0::BlockId;

#[async_trait]
impl StarknetReadRpcApiV0_10_2Server for Starknet {
    fn spec_version(&self) -> RpcResult<String> {
        Ok(RpcVersion::RPC_VERSION_0_10_2.to_string())
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
        Ok(estimate_fee::estimate_fee(self, request, simulation_flags, block_id).await?)
    }

    async fn estimate_message_fee(&self, message: MsgFromL1, block_id: BlockId) -> RpcResult<MessageFeeEstimate> {
        V0_10_0Impl::estimate_message_fee(self, message, block_id).await
    }

    fn get_block_with_receipts(
        &self,
        block_id: BlockId,
        response_flags: Option<Vec<ResponseFlag>>,
    ) -> RpcResult<StarknetGetBlockWithTxsAndReceiptsResult> {
        Ok(get_block_with_receipts::get_block_with_receipts(self, block_id, response_flags)?)
    }

    fn get_block_with_tx_hashes(&self, block_id: BlockId) -> RpcResult<MaybePreConfirmedBlockWithTxHashes> {
        V0_10_0Impl::get_block_with_tx_hashes(self, block_id)
    }

    fn get_block_with_txs(
        &self,
        block_id: BlockId,
        response_flags: Option<Vec<ResponseFlag>>,
    ) -> RpcResult<MaybePreConfirmedBlockWithTxsAndProofFacts> {
        Ok(get_block_with_txs::get_block_with_txs(self, block_id, response_flags)?)
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

    fn get_storage_at(
        &self,
        contract_address: Felt,
        key: Felt,
        block_id: BlockId,
        response_flags: Option<Vec<StorageResponseFlag>>,
    ) -> RpcResult<GetStorageAtResult> {
        Ok(get_storage_at::get_storage_at(self, contract_address, key, block_id, response_flags)?)
    }

    fn get_transaction_by_block_id_and_index(
        &self,
        block_id: BlockId,
        index: u64,
        response_flags: Option<Vec<ResponseFlag>>,
    ) -> RpcResult<TxnWithHashAndProofFacts> {
        Ok(get_transaction_by_block_id_and_index::get_transaction_by_block_id_and_index(
            self,
            block_id,
            index,
            response_flags,
        )?)
    }

    fn get_transaction_by_hash(
        &self,
        transaction_hash: Felt,
        response_flags: Option<Vec<ResponseFlag>>,
    ) -> RpcResult<TxnWithHashAndProofFacts> {
        Ok(get_transaction_by_hash::get_transaction_by_hash(self, transaction_hash, response_flags)?)
    }

    fn get_transaction_receipt(&self, transaction_hash: Felt) -> RpcResult<TxnReceiptWithBlockInfo> {
        V0_9_0Impl::get_transaction_receipt(self, transaction_hash)
    }

    async fn get_transaction_status(&self, transaction_hash: Felt) -> RpcResult<TxnFinalityAndExecutionStatus> {
        V0_9_0Impl::get_transaction_status(self, transaction_hash).await
    }

    fn get_state_update(
        &self,
        block_id: BlockId,
        contract_addresses: Option<Vec<Felt>>,
    ) -> RpcResult<MaybePreConfirmedStateUpdate> {
        Ok(get_state_update::get_state_update(self, block_id, contract_addresses)?)
    }

    fn get_messages_status(&self, transaction_hash: L1TxnHash) -> RpcResult<Vec<MessageStatus>> {
        V0_9_0Impl::get_messages_status(self, transaction_hash)
    }

    fn get_storage_proof(
        &self,
        block_id: BlockId,
        class_hashes: Option<Vec<Felt>>,
        contract_addresses: Option<Vec<Felt>>,
        contracts_storage_keys: Option<Vec<ContractStorageKeysItem>>,
    ) -> RpcResult<GetStorageProofResult> {
        get_storage_proof::get_storage_proof(self, block_id, class_hashes, contract_addresses, contracts_storage_keys)
    }

    fn get_compiled_casm(&self, class_hash: Felt) -> RpcResult<serde_json::Value> {
        V0_8_1Impl::get_compiled_casm(self, class_hash)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::rpc_test_setup;
    use jsonrpsee::types::ErrorCode;
    use mc_db::MadaraBackend;
    use mp_block::{
        header::{BlockTimestamp, GasPrices, PreconfirmedHeader},
        FullBlockWithoutCommitments, TransactionWithReceipt,
    };
    use mp_chain_config::{L1DataAvailabilityMode, StarknetVersion};
    use mp_receipt::{
        ExecutionResources, ExecutionResult, FeePayment, InvokeTransactionReceipt, PriceUnit, TransactionReceipt,
    };
    use mp_rpc::v0_10_2::{
        InvokeTxnWithProofFacts, MaybePreConfirmedBlockWithTxsAndProofFacts, ResponseFlag,
        StarknetGetBlockWithTxsAndReceiptsResult, TransactionAndReceipt, TxnWithProofFacts,
    };
    use mp_transactions::{
        DataAvailabilityMode, InvokeTransaction, InvokeTransactionV3, ResourceBoundsMapping, Transaction,
    };
    use rstest::rstest;
    use starknet_types_core::felt::Felt;
    use std::sync::Arc;

    fn add_block_with_invoke_v3(backend: &Arc<MadaraBackend>, tx_hash: Felt, proof_facts: Option<Vec<Felt>>) {
        let _block_hash = backend
            .write_access()
            .add_full_block_with_classes(
                &FullBlockWithoutCommitments {
                    header: PreconfirmedHeader {
                        block_number: 0,
                        sequencer_address: Felt::from_hex_unchecked("0xbabaa"),
                        block_timestamp: BlockTimestamp(43),
                        protocol_version: StarknetVersion::V0_13_1_1,
                        gas_prices: GasPrices {
                            eth_l1_gas_price: 123,
                            strk_l1_gas_price: 12,
                            eth_l1_data_gas_price: 44,
                            strk_l1_data_gas_price: 52,
                            eth_l2_gas_price: 0,
                            strk_l2_gas_price: 0,
                        },
                        l1_da_mode: L1DataAvailabilityMode::Blob,
                    },
                    state_diff: Default::default(),
                    transactions: vec![TransactionWithReceipt {
                        transaction: Transaction::Invoke(InvokeTransaction::V3(InvokeTransactionV3 {
                            sender_address: Felt::from_hex_unchecked("0x1234"),
                            calldata: vec![Felt::from_hex_unchecked("0x55")].into(),
                            signature: vec![].into(),
                            nonce: Felt::ZERO,
                            resource_bounds: ResourceBoundsMapping::default(),
                            tip: 0,
                            paymaster_data: vec![],
                            account_deployment_data: vec![],
                            nonce_data_availability_mode: DataAvailabilityMode::L1,
                            fee_data_availability_mode: DataAvailabilityMode::L1,
                            proof_facts,
                        })),
                        receipt: TransactionReceipt::Invoke(InvokeTransactionReceipt {
                            transaction_hash: tx_hash,
                            actual_fee: FeePayment { amount: Felt::ONE, unit: PriceUnit::Wei },
                            messages_sent: vec![],
                            events: vec![],
                            execution_resources: ExecutionResources::default(),
                            execution_result: ExecutionResult::Succeeded,
                        }),
                    }],
                    events: vec![],
                },
                &[],
                true,
            )
            .unwrap()
            .block_hash;
    }

    fn add_block_with_invoke_v3_proof_facts(backend: &Arc<MadaraBackend>) -> (Felt, Vec<Felt>) {
        let tx_hash = Felt::from_hex_unchecked("0x111");
        let proof_facts = vec![Felt::from_hex_unchecked("0xabc")];

        add_block_with_invoke_v3(backend, tx_hash, Some(proof_facts.clone()));

        (tx_hash, proof_facts)
    }

    fn add_block_with_invoke_v3_without_proof_facts(backend: &Arc<MadaraBackend>) -> Felt {
        let tx_hash = Felt::from_hex_unchecked("0x112");
        add_block_with_invoke_v3(backend, tx_hash, None);
        tx_hash
    }

    fn assert_proof_facts(tx: &TxnWithHashAndProofFacts, expected: Option<&[Felt]>) {
        let TxnWithProofFacts::Invoke(InvokeTxnWithProofFacts::V3(tx)) = &tx.transaction else {
            panic!("expected invoke v3 transaction");
        };
        assert_eq!(tx.proof_facts.as_deref(), expected);
    }

    #[rstest]
    fn test_response_flags_include_proof_facts(rpc_test_setup: (Arc<MadaraBackend>, Starknet)) {
        let (backend, rpc) = rpc_test_setup;
        let (tx_hash, proof_facts) = add_block_with_invoke_v3_proof_facts(&backend);

        let no_flags = StarknetReadRpcApiV0_10_2Server::get_transaction_by_hash(&rpc, tx_hash, None).unwrap();
        assert_proof_facts(&no_flags, None);

        let with_flags = StarknetReadRpcApiV0_10_2Server::get_transaction_by_hash(
            &rpc,
            tx_hash,
            Some(vec![ResponseFlag::IncludeProofFacts]),
        )
        .unwrap();
        assert_proof_facts(&with_flags, Some(&proof_facts));

        let by_index = StarknetReadRpcApiV0_10_2Server::get_transaction_by_block_id_and_index(
            &rpc,
            BlockId::Number(0),
            0,
            Some(vec![ResponseFlag::IncludeProofFacts]),
        )
        .unwrap();
        assert_proof_facts(&by_index, Some(&proof_facts));

        let block = StarknetReadRpcApiV0_10_2Server::get_block_with_txs(
            &rpc,
            BlockId::Number(0),
            Some(vec![ResponseFlag::IncludeProofFacts]),
        )
        .unwrap();
        let MaybePreConfirmedBlockWithTxsAndProofFacts::Block(block) = block else {
            panic!("expected confirmed block");
        };
        assert_proof_facts(&block.transactions[0], Some(&proof_facts));

        let receipts = StarknetReadRpcApiV0_10_2Server::get_block_with_receipts(
            &rpc,
            BlockId::Number(0),
            Some(vec![ResponseFlag::IncludeProofFacts]),
        )
        .unwrap();
        let StarknetGetBlockWithTxsAndReceiptsResult::Block(block) = receipts else {
            panic!("expected confirmed receipts block");
        };
        let TransactionAndReceipt { transaction, .. } = &block.transactions[0];
        let TxnWithProofFacts::Invoke(InvokeTxnWithProofFacts::V3(tx)) = transaction else {
            panic!("expected invoke v3 transaction");
        };
        assert_eq!(tx.proof_facts.as_deref(), Some(proof_facts.as_slice()));
    }

    #[rstest]
    fn test_response_flags_include_proof_facts_returns_empty_for_missing_facts(
        rpc_test_setup: (Arc<MadaraBackend>, Starknet),
    ) {
        let (backend, rpc) = rpc_test_setup;
        let tx_hash = add_block_with_invoke_v3_without_proof_facts(&backend);

        let no_flags = StarknetReadRpcApiV0_10_2Server::get_transaction_by_hash(&rpc, tx_hash, None).unwrap();
        assert_proof_facts(&no_flags, None);

        let with_flags = StarknetReadRpcApiV0_10_2Server::get_transaction_by_hash(
            &rpc,
            tx_hash,
            Some(vec![ResponseFlag::IncludeProofFacts]),
        )
        .unwrap();
        assert_proof_facts(&with_flags, Some(&[]));
    }

    #[rstest]
    fn test_get_storage_proof_invalid_storage_key_returns_invalid_params(
        rpc_test_setup: (Arc<MadaraBackend>, Starknet),
    ) {
        let (backend, rpc) = rpc_test_setup;
        add_block_with_invoke_v3_without_proof_facts(&backend);

        let err = StarknetReadRpcApiV0_10_2Server::get_storage_proof(
            &rpc,
            BlockId::Number(0),
            None,
            None,
            Some(vec![ContractStorageKeysItem {
                contract_address: Felt::ONE,
                storage_keys: vec!["not-a-felt".to_string()],
            }]),
        )
        .unwrap_err();

        assert_eq!(err.code(), ErrorCode::InvalidParams.code());
        assert_eq!(err.message(), "Invalid storage key: not-a-felt");
    }

    #[rstest]
    fn test_get_storage_proof_preconfirmed_block_id_returns_invalid_params(
        rpc_test_setup: (Arc<MadaraBackend>, Starknet),
    ) {
        let (backend, rpc) = rpc_test_setup;
        add_block_with_invoke_v3_without_proof_facts(&backend);

        let err = StarknetReadRpcApiV0_10_2Server::get_storage_proof(
            &rpc,
            BlockId::Tag(mp_rpc::v0_10_0::BlockTag::PreConfirmed),
            None,
            None,
            None,
        )
        .unwrap_err();

        assert_eq!(err.code(), ErrorCode::InvalidParams.code());
        assert_eq!(err.message(), "Invalid params");
    }
}
