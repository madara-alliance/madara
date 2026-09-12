//! Mutable state accumulated while the executor is building one block.

use super::*;
use crate::util::ExecutionStats;
use blockifier::blockifier::transaction_executor::TransactionExecutorError;
use blockifier::transaction::errors::{TransactionExecutionError, TransactionPreValidationError};
use mc_db::MadaraStorageRead;
use mp_transactions::validated::ValidatedTransaction;

fn is_future_nonce_error(error: &TransactionExecutorError) -> bool {
    matches!(error,
        TransactionExecutorError::TransactionExecutionError(
            TransactionExecutionError::TransactionPreValidationError(error)
        ) if matches!(error.as_ref(), TransactionPreValidationError::InvalidNonce {
            account_nonce, incoming_tx_nonce, ..
        } if incoming_tx_nonce > account_nonce)
    )
}

impl CurrentBlockState {
    /// Classifies against the whole parent chain, including blocks awaiting finalization.
    fn classify_class_update(&mut self, address: Felt, class_hash: Felt) -> anyhow::Result<ClassUpdateItem> {
        if self.deployed_contracts.contains(&address) {
            return Ok(ClassUpdateItem::ReplacedClass(class_hash));
        }

        let first_unconfirmed = self.backend.latest_confirmed_block_n().map_or(0, |n| n + 1);
        for block_n in (first_unconfirmed..self.block_number).rev() {
            if let Some(parent) = self.backend.block_view_on_preconfirmed(block_n) {
                if parent
                    .borrow_content()
                    .executed_transactions()
                    .any(|tx| tx.state_diff.contract_class_hashes.contains_key(&address))
                {
                    return Ok(ClassUpdateItem::ReplacedClass(class_hash));
                }
            }
        }

        // Read persisted state last: finalization may have removed a parent from the runtime
        // map while we inspected it. Its state is persisted before that removal.
        if let Some(parent_n) = self.block_number.checked_sub(1) {
            if self.backend.db.is_contract_deployed_at(parent_n, &address)? {
                return Ok(ClassUpdateItem::ReplacedClass(class_hash));
            }
        }
        self.deployed_contracts.insert(address);
        Ok(ClassUpdateItem::DeployedContract(class_hash))
    }

    /// Starts empty aggregation state for one executor block.
    pub fn new(backend: Arc<MadaraBackend>, block_number: u64) -> Self {
        Self {
            backend,
            block_number,
            consumed_core_contract_nonces: Default::default(),
            deployed_contracts: Default::default(),
            block_start_time: Instant::now(),
            accumulated_stats: Default::default(),
            last_execution_finished_at: None,
        }
    }
    /// Persists successful execution records into this block's durable preconfirmed entry.
    ///
    /// Persistence runs on Rayon because the backend write path is synchronous.
    async fn persist_executed_transactions(
        &self,
        executed: Vec<PreconfirmedExecutedTransaction>,
    ) -> anyhow::Result<()> {
        let backend = Arc::clone(&self.backend);
        let block_number = self.block_number;
        global_spawn_rayon_task(move || {
            backend
                .write_access()
                .append_to_preconfirmed(block_number, &executed, /* candidates */ [])
                .context("Appending to preconfirmed block")
        })
        .await
    }

    /// Records the aggregate execution result for one batch.
    ///
    /// Empty batches stay silent because they carry no useful production timing.
    fn log_batch_stats(&self, stats: &ExecutionStats) {
        if stats.n_executed == 0 {
            return;
        }
        tracing::debug!(
            txs_executed_in_batch = stats.n_executed,
            txs_added_to_block = stats.n_added_to_block,
            txs_reverted = stats.n_reverted,
            txs_rejected = stats.n_rejected,
            batch_exec_duration_ms = stats.exec_duration.as_secs_f64() * 1000.0,
            "🧮 Executed and added {} transaction(s) to the preconfirmed block at height {} - {:.3?}",
            stats.n_added_to_block,
            self.block_number,
            stats.exec_duration,
        );
        tracing::debug!("Tick stats {:?}", stats);
    }

    /// Converts one executor batch into durable preconfirmed transactions and appends them.
    ///
    /// Rejected executions are omitted, while consumed L1 nonces remain recorded even on revert.
    pub async fn append_batch(&mut self, mut batch: BatchExecutionResult) -> anyhow::Result<Vec<ValidatedTransaction>> {
        let mut executed = vec![];
        let mut deferred = vec![];

        for ((blockifier_exec_result, blockifier_tx), mut additional_info) in
            batch.blockifier_results.into_iter().zip(batch.executed_txs.txs).zip(batch.executed_txs.additional_info)
        {
            // A rejected predecessor does not consume its nonce. Its speculative
            // successors remain valid pending work, not permanently rejected txs.
            if additional_info.from_mempool && blockifier_exec_result.as_ref().is_err_and(is_future_nonce_error) {
                if let blockifier::transaction::transaction_execution::Transaction::Account(tx) = blockifier_tx {
                    deferred.push(ValidatedTransaction::from_starknet_api(
                        tx.tx,
                        additional_info.arrived_at,
                        additional_info.declared_class,
                        tx.execution_flags.charge_fee,
                    ));
                }
                continue;
            }
            if let Some(core_contract_nonce) = blockifier_tx.l1_handler_tx_nonce() {
                // Even when the l1 handler tx is reverted, we mark the nonce as consumed.
                self.consumed_core_contract_nonces
                    .insert(core_contract_nonce.to_felt().try_into().context("Invalid nonce while appending batch")?);
            }

            if let Ok((execution_info, state_diff)) = blockifier_exec_result {
                let declared_class = additional_info.declared_class.take().filter(|_| !execution_info.is_reverted());

                let receipt = from_blockifier_execution_info(&execution_info, &blockifier_tx);

                // Extract paid_fee_on_l1 from L1 handler transactions
                let paid_fee_on_l1 = match &blockifier_tx {
                    blockifier::transaction::transaction_execution::Transaction::L1Handler(l1_tx) => {
                        Some(l1_tx.paid_fee_on_l1.0)
                    }
                    _ => None,
                };

                // ponytail: finish borrowing the transaction before consuming it; no deep clone is needed.
                let converted_tx = TransactionWithHash::from(blockifier_tx);
                executed.push(PreconfirmedExecutedTransaction {
                    transaction: TransactionWithReceipt { transaction: converted_tx.transaction, receipt },
                    state_diff: TransactionStateUpdate {
                        nonces: state_diff
                            .nonces
                            .into_iter()
                            .map(|(contract_addr, nonce)| (contract_addr.to_felt(), nonce.to_felt()))
                            .collect(),
                        contract_class_hashes: state_diff
                            .class_hashes
                            .into_iter()
                            .map(|(contract_addr, class_hash)| {
                                let entry =
                                    self.classify_class_update(contract_addr.to_felt(), class_hash.to_felt())?;

                                Ok((contract_addr.to_felt(), entry))
                            })
                            .collect::<anyhow::Result<_>>()?,
                        storage_diffs: state_diff
                            .storage
                            .into_iter()
                            .map(|((contract_addr, key), value)| ((contract_addr.to_felt(), key.to_felt()), value))
                            .collect(),
                        declared_classes: declared_class
                            .iter()
                            .map(|class| {
                                (
                                    *class.class_hash(),
                                    class
                                        .as_sierra()
                                        .and_then(|class| {
                                            // Use canonical hash (v2 if present, else v1)
                                            let hash =
                                                class.info.compiled_class_hash_v2.or(class.info.compiled_class_hash)?;
                                            Some(DeclaredClassCompiledClass::Sierra(hash))
                                        })
                                        .unwrap_or(DeclaredClassCompiledClass::Legacy),
                                )
                            })
                            .collect(),
                    },
                    declared_class,
                    arrived_at: additional_info.arrived_at,
                    paid_fee_on_l1,
                })
            }
        }

        self.persist_executed_transactions(executed).await?;
        let stats = mem::take(&mut batch.stats);
        self.log_batch_stats(&stats);
        Ok(deferred)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mc_db::preconfirmed::{PreconfirmedBlock, PreconfirmedExecutedTransaction};
    use mp_block::{header::PreconfirmedHeader, TransactionWithReceipt};
    use mp_chain_config::ChainConfig;
    use mp_receipt::{InvokeTransactionReceipt, TransactionReceipt};
    use mp_transactions::{InvokeTransaction, InvokeTransactionV0, Transaction};

    #[rstest::rstest]
    #[case::future_mempool(true, 1, 2, true)]
    #[case::stale_mempool(true, 2, 1, false)]
    #[case::future_bypass(false, 1, 2, false)]
    #[tokio::test]
    async fn append_batch_requeues_only_future_mempool_nonce_errors(
        #[case] from_mempool: bool,
        #[case] account_nonce: u64,
        #[case] incoming_nonce: u64,
        #[case] requeue: bool,
    ) {
        use crate::util::AdditionalTxInfo;
        use mp_transactions::validated::TxTimestamp;
        use mp_transactions::InvokeTransactionV3;
        use starknet_api::core::Nonce;
        let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));
        backend.write_access().new_preconfirmed(PreconfirmedBlock::new(PreconfirmedHeader::default())).unwrap();
        let original = ValidatedTransaction {
            transaction: Transaction::Invoke(InvokeTransaction::V3(InvokeTransactionV3 {
                nonce: Felt::from(incoming_nonce),
                sender_address: Felt::ONE,
                ..Default::default()
            })),
            paid_fee_on_l1: None,
            contract_address: Felt::ONE,
            arrived_at: TxTimestamp(123),
            declared_class: None,
            hash: Felt::from(456u64),
            charge_fee: false,
        };
        let (tx, arrived_at, declared_class) = original.clone().into_blockifier_for_sequencing().unwrap();
        let error = TransactionPreValidationError::InvalidNonce {
            address: Felt::ONE.try_into().unwrap(),
            account_nonce: Nonce(Felt::from(account_nonce)),
            incoming_tx_nonce: Nonce(Felt::from(incoming_nonce)),
        };
        let batch = BatchExecutionResult {
            executed_txs: [(tx, AdditionalTxInfo { arrived_at, declared_class, from_mempool })].into_iter().collect(),
            blockifier_results: vec![Err(
                TransactionExecutionError::TransactionPreValidationError(Box::new(error)).into()
            )],
            stats: Default::default(),
            emitted_at: Instant::now(),
        };
        let mut current = CurrentBlockState::new(backend.clone(), 0);
        let deferred = current.append_batch(batch).await.unwrap();
        assert_eq!(deferred, if requeue { vec![original] } else { vec![] });
        assert_eq!(backend.block_view_on_preconfirmed(0).unwrap().num_executed_transactions(), 0);
    }

    #[test]
    fn non_nonce_error_is_not_deferred() {
        assert!(!is_future_nonce_error(&TransactionExecutorError::BlockFull));
    }

    #[test]
    fn replacement_sees_deployment_in_an_earlier_unconfirmed_block() {
        let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));
        let address = Felt::from(123u64);
        let first_class = Felt::from(456u64);
        let replacement = Felt::from(789u64);
        for n in 0..=2 {
            backend
                .write_access()
                .new_preconfirmed(PreconfirmedBlock::new(PreconfirmedHeader { block_number: n, ..Default::default() }))
                .unwrap();
        }
        let transaction = PreconfirmedExecutedTransaction {
            transaction: TransactionWithReceipt {
                transaction: Transaction::Invoke(InvokeTransaction::V0(InvokeTransactionV0::default())),
                receipt: TransactionReceipt::Invoke(InvokeTransactionReceipt::default()),
            },
            state_diff: TransactionStateUpdate {
                contract_class_hashes: [(address, ClassUpdateItem::DeployedContract(first_class))].into(),
                ..Default::default()
            },
            declared_class: None,
            arrived_at: Default::default(),
            paid_fee_on_l1: None,
        };
        backend.write_access().append_to_preconfirmed(0, &[transaction], []).unwrap();
        let mut current = CurrentBlockState::new(backend, 2);
        assert_eq!(
            current.classify_class_update(address, replacement).unwrap(),
            ClassUpdateItem::ReplacedClass(replacement)
        );
        assert!(current.deployed_contracts.is_empty(), "close normalization must preserve replacement classification");
    }

    #[test]
    fn deployment_followed_by_same_block_replacement_stays_a_block_deployment() {
        let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));
        let mut current = CurrentBlockState::new(backend, 0);
        let address = Felt::from(123u64);
        assert_eq!(
            current.classify_class_update(address, Felt::ONE).unwrap(),
            ClassUpdateItem::DeployedContract(Felt::ONE)
        );
        assert_eq!(
            current.classify_class_update(address, Felt::TWO).unwrap(),
            ClassUpdateItem::ReplacedClass(Felt::TWO)
        );
        assert!(current.deployed_contracts.contains(&address));
    }
}
