use crate::cli::block_production::BlockProductionParams;
use anyhow::Context;
use mc_block_production::{metrics::BlockProductionMetrics, BlockProductionHandle, BlockProductionTask};
use mc_db::MadaraBackend;
use mc_devnet::{
    storage_proof_bootstrap_account_class, ChainGenesisDescription, DevnetKeys, DevnetPredeployedContract,
};
use mc_mempool::Mempool;
use mc_settlement_client::SettlementClient;
use mc_submit_tx::SubmitTransaction;
use mp_convert::ToFelt;
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
                Ok(Some(mc_block_production::BlockProductionStateNotification::BatchExecuted)) => {
                    if self
                        .backend
                        .block_view_on_preconfirmed()
                        .map(|view| view.num_executed_transactions() >= expected_txs)
                        .unwrap_or(false)
                    {
                        break;
                    }
                }
                Ok(Some(mc_block_production::BlockProductionStateNotification::ClosedBlock)) => continue,
                Ok(None) => anyhow::bail!("Block production stopped before executing devnet bootstrap transactions"),
                Err(_) => anyhow::bail!("Timed out waiting for devnet bootstrap transactions to execute"),
            }
        }

        handle.close_block().await.context("Closing devnet bootstrap block")?;
        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                match notifications.recv().await {
                    Some(mc_block_production::BlockProductionStateNotification::ClosedBlock) => return anyhow::Ok(()),
                    Some(mc_block_production::BlockProductionStateNotification::BatchExecuted) => continue,
                    None => anyhow::bail!("Block production stopped before closing devnet bootstrap block"),
                }
            }
        })
        .await
        .context("Timed out waiting for devnet bootstrap block to close")??;

        anyhow::ensure!(
            self.backend.latest_confirmed_block_n() == Some(expected_latest),
            "Devnet bootstrap block closed at {:?}, expected {expected_latest}",
            self.backend.latest_confirmed_block_n()
        );

        Ok(())
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
    Felt::from_hex_unchecked("0x424f4f545354524150")
}

fn bootstrap_resource_bounds() -> ResourceBoundsMapping {
    ResourceBoundsMapping {
        l1_gas: ResourceBounds { max_amount: 0, max_price_per_unit: 0 },
        l2_gas: ResourceBounds { max_amount: 0, max_price_per_unit: 0 },
        l1_data_gas: ResourceBounds { max_amount: 0, max_price_per_unit: 0 },
    }
}
