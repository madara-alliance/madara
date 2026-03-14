use crate::versions::user::v0_10_0::StarknetReadRpcApiV0_10_0Server as V0_10_0Impl;
use crate::versions::user::v0_10_1::StarknetReadRpcApiV0_10_1Server;
use crate::versions::user::v0_8_1::StarknetReadRpcApiV0_8_1Server as V0_8_1Impl;
use crate::versions::user::v0_9_0::StarknetReadRpcApiV0_9_0Server as V0_9_0Impl;
use crate::{Starknet, StarknetRpcApiError};
use jsonrpsee::core::{async_trait, RpcResult};
use mp_block::MadaraMaybePreconfirmedBlockInfo;
use mp_chain_config::RpcVersion;
use mp_convert::Felt;
use mp_rpc::v0_10_1::{
    BlockHashAndNumber, BlockStatus, BlockWithReceipts, BlockWithTxsAndProofFacts, BroadcastedTxn, ContractStorageKeysItem,
    EventFilterWithPageRequest, EventsChunk, FeeEstimate, FunctionCall, GetStorageProofResult, L1TxnHash,
    MaybeDeprecatedContractClass, MaybePreConfirmedBlockWithTxHashes, MaybePreConfirmedBlockWithTxsAndProofFacts,
    MaybePreConfirmedStateUpdate, MessageFeeEstimate, MessageStatus, MsgFromL1, PreConfirmedBlockWithReceipts,
    PreConfirmedBlockWithTxsAndProofFacts, ResponseFlag, SimulationFlagForEstimateFee,
    StarknetGetBlockWithTxsAndReceiptsResult, SyncingStatus, TransactionAndReceipt, TxnFinalityAndExecutionStatus,
    TxnReceiptWithBlockInfo, TxnWithHashAndProofFacts,
};

// v0.10.1 specific implementation
pub mod get_events;
mod get_messages_status;

// Re-use BlockId from v0.10.0
use mp_rpc::v0_10_0::BlockId;

fn response_flags_include_proof_facts(response_flags: Option<Vec<ResponseFlag>>) -> bool {
    response_flags
        .map(|flags| flags.contains(&ResponseFlag::IncludeProofFacts))
        .unwrap_or(false)
}

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

    fn get_block_with_receipts(
        &self,
        block_id: BlockId,
        response_flags: Option<Vec<ResponseFlag>>,
    ) -> RpcResult<StarknetGetBlockWithTxsAndReceiptsResult> {
        let include_proof_facts = response_flags_include_proof_facts(response_flags);
        let view = self.resolve_block_view(block_id)?;
        let block_info = view.get_block_info().map_err(StarknetRpcApiError::from)?;

        let status = if view.is_preconfirmed() {
            BlockStatus::PreConfirmed
        } else if view.is_on_l1() {
            BlockStatus::AcceptedOnL1
        } else {
            BlockStatus::AcceptedOnL2
        };

        let transactions_with_receipts = view
            .get_executed_transactions(..)
            .map_err(StarknetRpcApiError::from)?
            .into_iter()
            .map(|tx| TransactionAndReceipt {
                receipt: tx.receipt.to_rpc_v0_9(status.into()),
                transaction: tx.transaction.to_rpc_v0_10_1(include_proof_facts),
            })
            .collect();

        Ok(match block_info {
            MadaraMaybePreconfirmedBlockInfo::Preconfirmed(block) => {
                StarknetGetBlockWithTxsAndReceiptsResult::PreConfirmed(PreConfirmedBlockWithReceipts {
                    transactions: transactions_with_receipts,
                    pre_confirmed_block_header: block.header.to_rpc_v0_9(),
                })
            }
            MadaraMaybePreconfirmedBlockInfo::Confirmed(block) => {
                StarknetGetBlockWithTxsAndReceiptsResult::Block(BlockWithReceipts {
                    transactions: transactions_with_receipts,
                    status,
                    block_header: block.to_rpc_v0_10(),
                })
            }
        })
    }

    fn get_block_with_tx_hashes(&self, block_id: BlockId) -> RpcResult<MaybePreConfirmedBlockWithTxHashes> {
        V0_10_0Impl::get_block_with_tx_hashes(self, block_id)
    }

    fn get_block_with_txs(
        &self,
        block_id: BlockId,
        response_flags: Option<Vec<ResponseFlag>>,
    ) -> RpcResult<MaybePreConfirmedBlockWithTxsAndProofFacts> {
        let include_proof_facts = response_flags_include_proof_facts(response_flags);
        let view = self.resolve_block_view(block_id)?;
        let block_info = view.get_block_info().map_err(StarknetRpcApiError::from)?;
        let txs: Vec<TxnWithHashAndProofFacts> = view
            .get_executed_transactions(..)
            .map_err(StarknetRpcApiError::from)?
            .into_iter()
            .map(|tx| TxnWithHashAndProofFacts {
                transaction: tx.transaction.to_rpc_v0_10_1(include_proof_facts),
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

        Ok(match block_info {
            MadaraMaybePreconfirmedBlockInfo::Preconfirmed(block) => {
                MaybePreConfirmedBlockWithTxsAndProofFacts::PreConfirmed(PreConfirmedBlockWithTxsAndProofFacts {
                    transactions: txs,
                    pre_confirmed_block_header: block.header.to_rpc_v0_9(),
                })
            }
            MadaraMaybePreconfirmedBlockInfo::Confirmed(block) => {
                MaybePreConfirmedBlockWithTxsAndProofFacts::Block(BlockWithTxsAndProofFacts {
                    transactions: txs,
                    status,
                    block_header: block.to_rpc_v0_10(),
                })
            }
        })
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
        response_flags: Option<Vec<ResponseFlag>>,
    ) -> RpcResult<TxnWithHashAndProofFacts> {
        let include_proof_facts = response_flags_include_proof_facts(response_flags);
        let view = self.resolve_block_view(block_id)?;
        let tx = view
            .get_executed_transaction(index)
            .map_err(StarknetRpcApiError::from)?
            .ok_or(StarknetRpcApiError::InvalidTxnIndex)?;
        Ok(TxnWithHashAndProofFacts {
            transaction: tx.transaction.to_rpc_v0_10_1(include_proof_facts),
            transaction_hash: *tx.receipt.transaction_hash(),
        })
    }

    fn get_transaction_by_hash(
        &self,
        transaction_hash: Felt,
        response_flags: Option<Vec<ResponseFlag>>,
    ) -> RpcResult<TxnWithHashAndProofFacts> {
        let include_proof_facts = response_flags_include_proof_facts(response_flags);
        let view = self.backend.view_on_latest();
        let res = view
            .find_transaction_by_hash(&transaction_hash)
            .map_err(StarknetRpcApiError::from)?
            .ok_or(StarknetRpcApiError::TxnHashNotFound)?;
        let tx = res.get_transaction().map_err(StarknetRpcApiError::from)?;
        Ok(TxnWithHashAndProofFacts {
            transaction: tx.transaction.to_rpc_v0_10_1(include_proof_facts),
            transaction_hash,
        })
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

    fn get_messages_status(&self, transaction_hash: L1TxnHash) -> RpcResult<Vec<MessageStatus>> {
        Ok(get_messages_status::get_messages_status(self, transaction_hash)?)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::rpc_test_setup;
    use mc_db::MadaraBackend;
    use mp_block::{
        header::{BlockTimestamp, GasPrices, PreconfirmedHeader},
        FullBlockWithoutCommitments, TransactionWithReceipt,
    };
    use mp_chain_config::{L1DataAvailabilityMode, StarknetVersion};
    use mp_receipt::{
        ExecutionResources, ExecutionResult, FeePayment, InvokeTransactionReceipt, PriceUnit, TransactionReceipt,
    };
    use mp_rpc::v0_10_1::{InvokeTxnWithProofFacts, TxnWithProofFacts};
    use mp_transactions::{DataAvailabilityMode, InvokeTransaction, InvokeTransactionV3, ResourceBoundsMapping, Transaction};
    use rstest::rstest;
    use starknet_types_core::felt::Felt;
    use std::sync::Arc;

    fn add_block_with_invoke_v3_proof_facts(backend: &Arc<MadaraBackend>) -> (Felt, Vec<Felt>) {
        let tx_hash = Felt::from_hex_unchecked("0x111");
        let proof_facts = vec![Felt::from_hex_unchecked("0xabc")];

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
                            proof_facts: Some(proof_facts.clone()),
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

        (tx_hash, proof_facts)
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

        let no_flags = StarknetReadRpcApiV0_10_1Server::get_transaction_by_hash(&rpc, tx_hash, None).unwrap();
        assert_proof_facts(&no_flags, None);

        let with_flags = StarknetReadRpcApiV0_10_1Server::get_transaction_by_hash(
            &rpc,
            tx_hash,
            Some(vec![ResponseFlag::IncludeProofFacts]),
        )
        .unwrap();
        assert_proof_facts(&with_flags, Some(&proof_facts));

        let by_index = StarknetReadRpcApiV0_10_1Server::get_transaction_by_block_id_and_index(
            &rpc,
            BlockId::Number(0),
            0,
            Some(vec![ResponseFlag::IncludeProofFacts]),
        )
        .unwrap();
        assert_proof_facts(&by_index, Some(&proof_facts));

        let block = StarknetReadRpcApiV0_10_1Server::get_block_with_txs(
            &rpc,
            BlockId::Number(0),
            Some(vec![ResponseFlag::IncludeProofFacts]),
        )
        .unwrap();
        let MaybePreConfirmedBlockWithTxsAndProofFacts::Block(block) = block else {
            panic!("expected confirmed block");
        };
        assert_proof_facts(&block.transactions[0], Some(&proof_facts));

        let receipts = StarknetReadRpcApiV0_10_1Server::get_block_with_receipts(
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

}
