use crate::cli::block_production::BlockProductionParams;
use anyhow::Context;
use mc_block_production::{
    metrics::BlockProductionMetrics, BlockProductionHandle, BlockProductionStateNotification, BlockProductionTask,
};
use mc_db::MadaraBackend;
use mc_devnet::{
    storage_proof_bootstrap_account_class, ChainGenesisDescription, DevnetKeys, DevnetPredeployedContract,
};
use mc_mempool::Mempool;
use mc_settlement_client::SettlementClient;
use mc_submit_tx::SubmitTransaction;
use mp_convert::ToFelt;
use mp_receipt::ExecutionResult;
use mp_rpc::v0_9_0::{
    BroadcastedDeclareTxn, BroadcastedDeclareTxnV3, BroadcastedDeployAccountTxn, BroadcastedTxn, DaMode,
    DeployAccountTxnV3, ResourceBounds, ResourceBoundsMapping,
};
use mp_transactions::validated::TxTimestamp;
use mp_transactions::IntoStarknetApiExt;
use mp_utils::service::{MadaraServiceId, PowerOfTwo, Service, ServiceContext, ServiceId, ServiceRunner};
use mp_utils::AbortOnDrop;
use starknet_types_core::felt::Felt;
use std::{io::Write, sync::Arc, time::Duration};

pub struct BlockProductionService {
    backend: Arc<MadaraBackend>,
    task: Option<BlockProductionTask>,
    mempool: Arc<Mempool>,
    metrics: Arc<BlockProductionMetrics>,
    l1_client: Arc<dyn SettlementClient>,
    discard_preconfirmed_on_startup: bool,
    n_devnet_contracts: u64,
    devnet_storage_proof_bootstrap: bool,
    disabled: bool,
}

impl BlockProductionService {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        config: &BlockProductionParams,
        backend: &Arc<MadaraBackend>,
        mempool: Arc<mc_mempool::Mempool>,
        l1_client: Arc<dyn SettlementClient>,
        no_charge_fee: bool,
    ) -> anyhow::Result<Self> {
        let metrics = Arc::new(BlockProductionMetrics::register());

        Ok(Self {
            backend: backend.clone(),
            mempool: mempool.clone(),
            metrics: metrics.clone(),
            l1_client: l1_client.clone(),
            discard_preconfirmed_on_startup: config.discard_preconfirmed_on_startup,
            task: Some(BlockProductionTask::new(
                backend.clone(),
                mempool,
                metrics.clone(),
                l1_client,
                no_charge_fee,
                config.discard_preconfirmed_on_startup,
            )),
            n_devnet_contracts: config.devnet_contracts,
            devnet_storage_proof_bootstrap: config.devnet_storage_proof_bootstrap,
            disabled: config.block_production_disabled,
        })
    }
}

#[async_trait::async_trait]
impl Service for BlockProductionService {
    #[tracing::instrument(skip(self, runner), fields(module = "BlockProductionService"))]
    async fn start<'a>(&mut self, runner: ServiceRunner<'a>) -> anyhow::Result<()> {
        let block_production_task = self.task.take().context("Service already started")?;
        if !self.disabled {
            runner.service_loop(move |ctx| block_production_task.run(ctx));
        }

        Ok(())
    }
}

impl ServiceId for BlockProductionService {
    #[inline(always)]
    fn svc_id(&self) -> PowerOfTwo {
        MadaraServiceId::BlockProduction.svc_id()
    }
}

impl BlockProductionService {
    /// Initializes the genesis state of a devnet. This is needed for local sequencers.
    ///
    /// This methods was made external to [Service::start] as it needs to be
    /// called on node startup even if sequencer block production is not yet
    /// enabled. This happens during warp updates on a local sequencer.
    pub async fn setup_devnet(&self) -> anyhow::Result<()> {
        let Self { backend, n_devnet_contracts, devnet_storage_proof_bootstrap, .. } = self;

        let keys = if backend.latest_confirmed_block_n().is_none() {
            // deploy devnet genesis
            tracing::info!("⛏️  Deploying devnet genesis block");

            let contracts;
            if *devnet_storage_proof_bootstrap {
                let mut genesis_config = ChainGenesisDescription::empty();
                contracts = genesis_config
                    .add_devnet_contracts_for_storage_proof_bootstrap(*n_devnet_contracts)
                    .context("Failed to add devnet contracts")?;
                contracts.save_to_db(backend)?;
                genesis_config
                    .build_and_store(backend)
                    .await
                    .context("Building and storing empty devnet genesis block")?;
                self.run_storage_proof_bootstrap_transactions(&contracts)
                    .await
                    .context("Executing storage-proof-compatible devnet bootstrap transactions")?;
            } else {
                let mut genesis_config =
                    ChainGenesisDescription::base_config().context("Failed to create base genesis config")?;
                contracts = genesis_config
                    .add_devnet_contracts(*n_devnet_contracts)
                    .context("Failed to add devnet contracts")?;
                contracts.save_to_db(backend)?;
                genesis_config.build_and_store(backend).await.context("Building and storing genesis block")?;
            }

            contracts
        } else {
            DevnetKeys::from_db(backend).context("Getting the devnet predeployed contract keys and balances")?
        };

        // display devnet welcome message :)
        // we display it to stdout instead of stderr
        let msg = format!("{}", keys);
        std::io::stdout().write(msg.as_bytes()).context("Writing devnet welcome message to stdout")?;

        anyhow::Ok(())
    }

    async fn run_storage_proof_bootstrap_transactions(&self, contracts: &DevnetKeys) -> anyhow::Result<()> {
        if contracts.0.is_empty() {
            return Ok(());
        }

        let mut block_production = BlockProductionTask::new(
            self.backend.clone(),
            self.mempool.clone(),
            self.metrics.clone(),
            self.l1_client.clone(),
            true, /* no_charge_fee */
            self.discard_preconfirmed_on_startup,
        );
        let mut notifications = block_production.subscribe_state_notifications();
        let handle = block_production.handle();
        let ctx = ServiceContext::new();
        let task = AbortOnDrop::spawn(async move { block_production.run(ctx).await });

        handle
            .submit_declare_transaction(self.make_bootstrap_account_declare_tx()?)
            .await
            .context("Submitting devnet bootstrap account class declaration")?;
        self.close_bootstrap_block(&handle, &mut notifications, 1, 1).await?;

        for contract in &contracts.0 {
            handle
                .submit_deploy_account_transaction(self.make_deploy_account_tx(contract)?)
                .await
                .context("Submitting devnet bootstrap deploy-account transaction")?;
        }
        self.close_bootstrap_block(&handle, &mut notifications, 2, contracts.0.len()).await?;

        drop(task);
        Ok(())
    }

    async fn close_bootstrap_block(
        &self,
        handle: &BlockProductionHandle,
        notifications: &mut tokio::sync::mpsc::UnboundedReceiver<mc_block_production::BlockProductionStateNotification>,
        expected_latest: u64,
        expected_txs: usize,
    ) -> anyhow::Result<()> {
        loop {
            match tokio::time::timeout(Duration::from_secs(30), notifications.recv()).await {
                Ok(Some(BlockProductionStateNotification::BatchExecuted)) => {
                    if self
                        .backend
                        .block_view_on_preconfirmed()
                        .map(|view| view.num_executed_transactions() >= expected_txs)
                        .unwrap_or(false)
                    {
                        break;
                    }
                }
                Ok(Some(BlockProductionStateNotification::ClosedBlock)) => {
                    if self.bootstrap_block_closed_with_expected_transactions(expected_latest, expected_txs)? {
                        return Ok(());
                    }
                }
                Ok(None) => anyhow::bail!("Block production stopped before executing devnet bootstrap transactions"),
                Err(_) => anyhow::bail!("Timed out waiting for devnet bootstrap transactions to execute"),
            }
        }

        if self.bootstrap_block_closed_with_expected_transactions(expected_latest, expected_txs)? {
            return Ok(());
        }

        handle.close_block().await.context("Closing devnet bootstrap block")?;
        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                match notifications.recv().await {
                    Some(BlockProductionStateNotification::ClosedBlock) => return anyhow::Ok(()),
                    Some(BlockProductionStateNotification::BatchExecuted) => continue,
                    None => anyhow::bail!("Block production stopped before closing devnet bootstrap block"),
                }
            }
        })
        .await
        .context("Timed out waiting for devnet bootstrap block to close")??;

        anyhow::ensure!(
            self.bootstrap_block_closed_with_expected_transactions(expected_latest, expected_txs)?,
            "Devnet bootstrap block #{expected_latest} was not confirmed after close notification"
        );

        Ok(())
    }

    fn bootstrap_block_closed_with_expected_transactions(
        &self,
        expected_latest: u64,
        expected_txs: usize,
    ) -> anyhow::Result<bool> {
        let Some(latest) = self.backend.latest_confirmed_block_n() else {
            return Ok(false);
        };
        if latest < expected_latest {
            return Ok(false);
        }

        let transactions = self
            .backend
            .block_view_on_confirmed(expected_latest)
            .with_context(|| format!("Devnet bootstrap block #{expected_latest} should be confirmed"))?
            .get_executed_transactions(..)
            .with_context(|| format!("Reading devnet bootstrap block #{expected_latest} transactions"))?;
        let actual_txs = transactions.len();
        anyhow::ensure!(
            actual_txs == expected_txs,
            "Devnet bootstrap block #{expected_latest} closed with {actual_txs} transactions, expected {expected_txs}; \
             the configured block_time may be too short for storage-proof bootstrap"
        );
        if let Some((tx_index, tx)) =
            transactions.iter().enumerate().find(|(_, tx)| tx.receipt.execution_result() != ExecutionResult::Succeeded)
        {
            anyhow::bail!(
                "Devnet bootstrap block #{expected_latest} transaction #{tx_index} reverted: {:?}",
                tx.receipt.execution_result()
            );
        }
        anyhow::ensure!(
            latest == expected_latest,
            "Devnet bootstrap advanced to block #{latest} while waiting for block #{expected_latest} to close"
        );

        Ok(true)
    }

    fn make_bootstrap_account_declare_tx(&self) -> anyhow::Result<BroadcastedDeclareTxn> {
        let account_class =
            storage_proof_bootstrap_account_class().context("Loading storage-proof bootstrap account class")?;

        Ok(BroadcastedDeclareTxn::V3(BroadcastedDeclareTxnV3 {
            sender_address: bootstrap_address(),
            compiled_class_hash: account_class.compiled_class_hash,
            signature: vec![].into(),
            nonce: Felt::ZERO,
            contract_class: account_class.contract_class.into(),
            resource_bounds: bootstrap_resource_bounds(),
            tip: 0,
            paymaster_data: vec![],
            account_deployment_data: vec![],
            nonce_data_availability_mode: DaMode::L1,
            fee_data_availability_mode: DaMode::L1,
        }))
    }

    fn make_deploy_account_tx(
        &self,
        contract: &DevnetPredeployedContract,
    ) -> anyhow::Result<BroadcastedDeployAccountTxn> {
        let mut tx = BroadcastedDeployAccountTxn::V3(DeployAccountTxnV3 {
            signature: vec![].into(),
            nonce: Felt::ZERO,
            contract_address_salt: Felt::ZERO,
            constructor_calldata: vec![contract.pubkey],
            class_hash: contract.class_hash,
            resource_bounds: bootstrap_resource_bounds(),
            tip: 0,
            paymaster_data: vec![],
            nonce_data_availability_mode: DaMode::L1,
            fee_data_availability_mode: DaMode::L1,
        });

        self.sign_deploy_account_tx(&mut tx, contract)?;
        Ok(tx)
    }

    fn sign_deploy_account_tx(
        &self,
        tx: &mut BroadcastedDeployAccountTxn,
        contract: &DevnetPredeployedContract,
    ) -> anyhow::Result<()> {
        let api_tx = BroadcastedTxn::DeployAccount(tx.clone())
            .into_validated_tx(
                self.backend.chain_config().chain_id.to_felt(),
                self.backend.chain_config().latest_protocol_version,
                TxTimestamp::now(),
            )
            .context("Converting deploy-account bootstrap transaction")?;
        let signature = contract.secret.sign(&api_tx.hash).context("Signing deploy-account bootstrap transaction")?;

        let tx_signature = match tx {
            BroadcastedDeployAccountTxn::V1(tx) => &mut tx.signature,
            BroadcastedDeployAccountTxn::V3(tx) => &mut tx.signature,
            _ => unreachable!("the deploy account tx is not query only"),
        };
        *tx_signature = vec![signature.r, signature.s].into();
        Ok(())
    }

    pub fn handle(&self) -> BlockProductionHandle {
        self.task.as_ref().expect("Service started").handle()
    }
}

fn bootstrap_address() -> Felt {
    // SNOS accepts this undeployed BOOTSTRAP sender only for the storage-proof bootstrap declare:
    // it uses nonce 0, zero resource bounds, and the temporary producer runs with fee charging disabled.
    Felt::from_hex_unchecked("0x424f4f545354524150")
}

fn bootstrap_resource_bounds() -> ResourceBoundsMapping {
    ResourceBoundsMapping {
        l1_gas: ResourceBounds { max_amount: 0, max_price_per_unit: 0 },
        l2_gas: ResourceBounds { max_amount: 0, max_price_per_unit: 0 },
        l1_data_gas: ResourceBounds { max_amount: 0, max_price_per_unit: 0 },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mc_mempool::{Mempool, MempoolConfig};
    use mc_settlement_client::L1SyncDisabledClient;
    use mp_chain_config::ChainConfig;
    use mp_receipt::{ExecutionResult, TransactionReceipt};
    use std::sync::Arc;

    #[tokio::test]
    async fn storage_proof_bootstrap_transactions_create_declare_and_deploy_blocks() {
        let chain_config = Arc::new(ChainConfig::madara_devnet());
        let backend = MadaraBackend::open_for_testing(Arc::clone(&chain_config));
        backend.set_l1_gas_quote_for_testing();

        let mut genesis = ChainGenesisDescription::empty();
        let contracts = genesis.add_devnet_contracts_for_storage_proof_bootstrap(2).unwrap();
        contracts.save_to_db(&backend).unwrap();
        genesis.build_and_store(&backend).await.unwrap();

        let service = BlockProductionService {
            backend: Arc::clone(&backend),
            task: None,
            mempool: Arc::new(Mempool::new(Arc::clone(&backend), MempoolConfig::default())),
            metrics: Arc::new(BlockProductionMetrics::register()),
            l1_client: Arc::new(L1SyncDisabledClient),
            discard_preconfirmed_on_startup: false,
            n_devnet_contracts: contracts.0.len() as u64,
            devnet_storage_proof_bootstrap: true,
            disabled: false,
        };

        service.run_storage_proof_bootstrap_transactions(&contracts).await.unwrap();

        assert_eq!(backend.latest_confirmed_block_n(), Some(2));
        assert!(!backend.has_preconfirmed_block());

        let account_class = storage_proof_bootstrap_account_class().unwrap();
        let block_1 = backend.block_view_on_confirmed(1).unwrap();
        let block_1_transactions = block_1.get_executed_transactions(..).unwrap();
        assert_eq!(block_1_transactions.len(), 1);
        assert_eq!(block_1_transactions[0].receipt.execution_result(), ExecutionResult::Succeeded);
        assert!(matches!(block_1_transactions[0].receipt, TransactionReceipt::Declare(_)));

        let block_1_state_diff = block_1.get_state_diff().unwrap();
        assert_eq!(block_1_state_diff.declared_classes.len(), 1);
        assert_eq!(block_1_state_diff.declared_classes[0].class_hash, account_class.class_hash);
        assert_eq!(block_1_state_diff.declared_classes[0].compiled_class_hash, account_class.compiled_class_hash);

        let block_2 = backend.block_view_on_confirmed(2).unwrap();
        let block_2_transactions = block_2.get_executed_transactions(..).unwrap();
        assert_eq!(block_2_transactions.len(), contracts.0.len());
        assert!(block_2_transactions.iter().all(|tx| {
            tx.receipt.execution_result() == ExecutionResult::Succeeded
                && matches!(tx.receipt, TransactionReceipt::DeployAccount(_))
        }));

        let block_2_state_diff = block_2.get_state_diff().unwrap();
        assert_eq!(block_2_state_diff.deployed_contracts.len(), contracts.0.len());
        for contract in &contracts.0 {
            assert!(block_2_state_diff
                .deployed_contracts
                .iter()
                .any(|deployed| deployed.address == contract.address && deployed.class_hash == contract.class_hash));
        }
    }

    #[tokio::test]
    async fn close_bootstrap_block_accepts_already_closed_expected_block() {
        let mut chain_config = ChainConfig::madara_devnet();
        chain_config.block_time = Duration::from_millis(1);
        chain_config.no_empty_blocks = true;
        let chain_config = Arc::new(chain_config);
        let backend = MadaraBackend::open_for_testing(Arc::clone(&chain_config));
        backend.set_l1_gas_quote_for_testing();

        ChainGenesisDescription::empty().build_and_store(&backend).await.unwrap();

        let service = BlockProductionService {
            backend: Arc::clone(&backend),
            task: None,
            mempool: Arc::new(Mempool::new(Arc::clone(&backend), MempoolConfig::default())),
            metrics: Arc::new(BlockProductionMetrics::register()),
            l1_client: Arc::new(L1SyncDisabledClient),
            discard_preconfirmed_on_startup: false,
            n_devnet_contracts: 0,
            devnet_storage_proof_bootstrap: true,
            disabled: false,
        };

        let mut block_production = BlockProductionTask::new(
            Arc::clone(&backend),
            Arc::clone(&service.mempool),
            Arc::clone(&service.metrics),
            Arc::clone(&service.l1_client),
            true,
            false,
        );
        let mut notifications = block_production.subscribe_state_notifications();
        let handle = block_production.handle();
        let ctx = ServiceContext::new();
        let task = AbortOnDrop::spawn(async move { block_production.run(ctx).await });

        handle.submit_declare_transaction(service.make_bootstrap_account_declare_tx().unwrap()).await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;

        service.close_bootstrap_block(&handle, &mut notifications, 1, 1).await.unwrap();
        assert_eq!(backend.latest_confirmed_block_n(), Some(1));

        drop(task);
    }
}
