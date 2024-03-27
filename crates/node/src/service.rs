//! Service and ServiceFactory implementation. Specialized wrapper over substrate service.

use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use deoxys_runtime::{self, Hash, RuntimeApi, SealingMode, StarknetHasher};
use futures::channel::mpsc;
use futures::future;
use futures::future::BoxFuture;
use futures::prelude::*;
use mc_db::DeoxysBackend;
use mc_genesis_data_provider::OnDiskGenesisConfig;
use mc_mapping_sync::MappingSyncWorker;
use mc_storage::overrides_handle;
use mc_sync::fetch::fetchers::FetchConfig;
use mc_sync::starknet_sync_worker;
use mp_block::state_update::StateUpdateWrapper;
use mp_block::DeoxysBlock;
use mp_contract::class::ClassUpdateWrapper;
use mp_sequencer_address::{
    InherentDataProvider as SeqAddrInherentDataProvider, DEFAULT_SEQUENCER_ADDRESS, SEQ_ADDR_STORAGE_KEY,
};
use mp_types::block::DBlockT;
use parity_scale_codec::Encode;
use prometheus_endpoint::Registry;
use reqwest::Url;
use sc_basic_authorship::ProposerFactory;
use sc_client_api::{Backend, BlockBackend, BlockchainEvents, HeaderBackend};
use sc_consensus::{BasicQueue, BlockImportParams};
use sc_consensus_aura::{SlotProportion, StartAuraParams};
use sc_consensus_grandpa::{GrandpaBlockImport, SharedVoterState};
use sc_consensus_manual_seal::{ConsensusDataProvider, Error};
pub use sc_executor::NativeElseWasmExecutor;
use sc_service::error::Error as ServiceError;
use sc_service::{new_db_backend, Configuration, TaskManager, WarpSyncParams};
use sc_telemetry::{Telemetry, TelemetryHandle, TelemetryWorker};
use sc_transaction_pool::FullPool;
use sc_transaction_pool_api::OffchainTransactionPoolFactory;
use sp_api::offchain::OffchainStorage;
use sp_api::{ConstructRuntimeApi, ProvideRuntimeApi};
use sp_consensus_aura::sr25519::AuthorityPair as AuraPair;
use sp_inherents::InherentData;
use sp_offchain::STORAGE_PREFIX;
use sp_runtime::testing::Digest;
use sp_runtime::traits::Block as BlockT;
use sp_runtime::DigestItem;

use crate::genesis_block::MadaraGenesisBlockBuilder;
use crate::rpc::StarknetDeps;
use crate::starknet::{db_config_dir, MadaraBackend};
// Our native executor instance.
pub struct ExecutorDispatch;

const MADARA_TASK_GROUP: &str = "madara";

impl sc_executor::NativeExecutionDispatch for ExecutorDispatch {
    /// Only enable the benchmarking host functions when we actually want to
    /// benchmark.
    #[cfg(feature = "runtime-benchmarks")]
    type ExtendHostFunctions = frame_benchmarking::benchmarking::HostFunctions;
    /// Otherwise we only use the default Substrate host functions.
    #[cfg(not(feature = "runtime-benchmarks"))]
    type ExtendHostFunctions = ();

    fn dispatch(method: &str, data: &[u8]) -> Option<Vec<u8>> {
        deoxys_runtime::api::dispatch(method, data)
    }

    fn native_version() -> sc_executor::NativeVersion {
        deoxys_runtime::native_version()
    }
}

pub type FullClient = sc_service::TFullClient<DBlockT, RuntimeApi, NativeElseWasmExecutor<ExecutorDispatch>>;
type FullBackend = sc_service::TFullBackend<DBlockT>;
type FullSelectChain = sc_consensus::LongestChain<FullBackend, DBlockT>;

type BasicImportQueue = sc_consensus::DefaultImportQueue<DBlockT>;
type BoxBlockImport = sc_consensus::BoxBlockImport<DBlockT>;

/// The minimum period of blocks on which justifications will be
/// imported and generated.
const GRANDPA_JUSTIFICATION_PERIOD: u32 = 512;

#[allow(clippy::type_complexity)]
pub fn new_partial<BIQ>(
    config: &Configuration,
    build_import_queue: BIQ,
    cache_more_things: bool,
    genesis_block: DeoxysBlock,
) -> Result<
    sc_service::PartialComponents<
        FullClient,
        FullBackend,
        FullSelectChain,
        sc_consensus::DefaultImportQueue<DBlockT>,
        sc_transaction_pool::FullPool<DBlockT, FullClient>,
        (
            BoxBlockImport,
            sc_consensus_grandpa::LinkHalf<DBlockT, FullClient, FullSelectChain>,
            Option<Telemetry>,
            Arc<MadaraBackend>,
        ),
    >,
    ServiceError,
>
where
    RuntimeApi: ConstructRuntimeApi<DBlockT, FullClient>,
    RuntimeApi: Send + Sync + 'static,
    BIQ: FnOnce(
        Arc<FullClient>,
        &Configuration,
        &TaskManager,
        Option<TelemetryHandle>,
        GrandpaBlockImport<FullBackend, DBlockT, FullClient, FullSelectChain>,
        Arc<MadaraBackend>,
    ) -> Result<(BasicImportQueue, BoxBlockImport), ServiceError>,
{
    let telemetry = config
        .telemetry_endpoints
        .clone()
        .filter(|x| !x.is_empty())
        .map(|endpoints| -> Result<_, sc_telemetry::Error> {
            let worker = TelemetryWorker::new(16)?;
            let telemetry = worker.handle().new_telemetry(endpoints);
            Ok((worker, telemetry))
        })
        .transpose()?;

    let executor = sc_service::new_native_or_wasm_executor(config);

    let backend = new_db_backend(config.db_config())?;

    let genesis_block_builder = MadaraGenesisBlockBuilder::<DBlockT, _, _>::new(
        config.chain_spec.as_storage_builder(),
        true,
        backend.clone(),
        executor.clone(),
        genesis_block,
    )
    .unwrap();

    let (client, backend, keystore_container, task_manager) = sc_service::new_full_parts_with_genesis_builder::<
        DBlockT,
        RuntimeApi,
        _,
        MadaraGenesisBlockBuilder<DBlockT, FullBackend, NativeElseWasmExecutor<ExecutorDispatch>>,
    >(
        config,
        telemetry.as_ref().map(|(_, telemetry)| telemetry.handle()),
        executor,
        backend,
        genesis_block_builder,
    )?;

    let client = Arc::new(client);

    let telemetry = telemetry.map(|(worker, telemetry)| {
        task_manager.spawn_handle().spawn("telemetry", None, worker.run());
        telemetry
    });

    let select_chain = sc_consensus::LongestChain::new(backend.clone());

    let transaction_pool = sc_transaction_pool::BasicPool::new_full(
        config.transaction_pool.clone(),
        config.role.is_authority().into(),
        config.prometheus_registry(),
        task_manager.spawn_essential_handle(),
        client.clone(),
    );

    let (grandpa_block_import, grandpa_link) = sc_consensus_grandpa::block_import(
        client.clone(),
        GRANDPA_JUSTIFICATION_PERIOD,
        &client as &Arc<_>,
        select_chain.clone(),
        telemetry.as_ref().map(|x| x.handle()),
    )?;

    let deoxys_backend = DeoxysBackend::open(&config.database, &db_config_dir(config), cache_more_things)?;

    let (import_queue, block_import) = build_import_queue(
        client.clone(),
        config,
        &task_manager,
        telemetry.as_ref().map(|x| x.handle()),
        grandpa_block_import,
        // TODO: use `DeoxysBackend` import instead
        Arc::clone(deoxys_backend),
    )?;

    Ok(sc_service::PartialComponents {
        client,
        backend,
        task_manager,
        import_queue,
        keystore_container,
        select_chain,
        transaction_pool,
        other: (block_import, grandpa_link, telemetry, Arc::clone(deoxys_backend)),
    })
}

/// Build the import queue for the template runtime (aura + grandpa).
pub fn build_aura_grandpa_import_queue(
    client: Arc<FullClient>,
    config: &Configuration,
    task_manager: &TaskManager,
    telemetry: Option<TelemetryHandle>,
    grandpa_block_import: GrandpaBlockImport<FullBackend, DBlockT, FullClient, FullSelectChain>,
    _madara_backend: Arc<MadaraBackend>,
) -> Result<(BasicImportQueue, BoxBlockImport), ServiceError>
where
    RuntimeApi: ConstructRuntimeApi<DBlockT, FullClient>,
    RuntimeApi: Send + Sync + 'static,
{
    let slot_duration = sc_consensus_aura::slot_duration(&*client)?;

    let create_inherent_data_providers = move |_, ()| async move {
        let timestamp = sp_timestamp::InherentDataProvider::from_system_time();
        let slot = sp_consensus_aura::inherents::InherentDataProvider::from_timestamp_and_slot_duration(
            *timestamp,
            slot_duration,
        );
        Ok((slot, timestamp))
    };

    let import_queue =
        sc_consensus_aura::import_queue::<AuraPair, _, _, _, _, _>(sc_consensus_aura::ImportQueueParams {
            block_import: grandpa_block_import.clone(),
            justification_import: Some(Box::new(grandpa_block_import.clone())),
            client,
            create_inherent_data_providers,
            spawner: &task_manager.spawn_essential_handle(),
            registry: config.prometheus_registry(),
            check_for_equivocation: Default::default(),
            telemetry,
            compatibility_mode: sc_consensus_aura::CompatibilityMode::None,
        })
        .map_err::<ServiceError, _>(Into::into)?;

    Ok((import_queue, Box::new(grandpa_block_import)))
}

/// Build the import queue for the template runtime (manual seal).
pub fn build_manual_seal_import_queue(
    client: Arc<FullClient>,
    config: &Configuration,
    task_manager: &TaskManager,
    _telemetry: Option<TelemetryHandle>,
    _grandpa_block_import: GrandpaBlockImport<FullBackend, DBlockT, FullClient, FullSelectChain>,
    _madara_backend: Arc<MadaraBackend>,
) -> Result<(BasicImportQueue, BoxBlockImport), ServiceError>
where
    RuntimeApi: ConstructRuntimeApi<DBlockT, FullClient>,
    RuntimeApi: Send + Sync + 'static,
{
    Ok((
        sc_consensus_manual_seal::import_queue(
            Box::new(client.clone()),
            &task_manager.spawn_essential_handle(),
            config.prometheus_registry(),
        ),
        Box::new(client),
    ))
}

/// Builds a new service for a full client.
///
/// # Arguments
///
/// - `cache`: whether more information should be cached when storing the block in the database.
pub fn new_full(
    config: Configuration,
    sealing: SealingMode,
    rpc_port: u16,
    l1_url: Url,
    cache_more_things: bool,
    fetch_config: FetchConfig,
    genesis_block: DeoxysBlock,
) -> Result<TaskManager, ServiceError> {
    let build_import_queue =
        if sealing.is_default() { build_aura_grandpa_import_queue } else { build_manual_seal_import_queue };

    let sc_service::PartialComponents {
        client,
        backend,
        mut task_manager,
        import_queue,
        keystore_container,
        select_chain,
        transaction_pool,
        other: (block_import, grandpa_link, mut telemetry, madara_backend),
    } = new_partial(&config, build_import_queue, cache_more_things, genesis_block)?;

    let mut net_config = sc_network::config::FullNetworkConfiguration::new(&config.network);

    let grandpa_protocol_name = sc_consensus_grandpa::protocol_standard_name(
        &client.block_hash(0).ok().flatten().expect("Genesis block exists; qed"),
        &config.chain_spec,
    );

    let warp_sync_params = if sealing.is_default() {
        net_config
            .add_notification_protocol(sc_consensus_grandpa::grandpa_peers_set_config(grandpa_protocol_name.clone()));
        let warp_sync = Arc::new(sc_consensus_grandpa::warp_proof::NetworkProvider::new(
            backend.clone(),
            grandpa_link.shared_authority_set().clone(),
            Vec::default(),
        ));
        Some(WarpSyncParams::WithProvider(warp_sync))
    } else {
        None
    };

    let (network, system_rpc_tx, tx_handler_controller, network_starter, sync_service) =
        sc_service::build_network(sc_service::BuildNetworkParams {
            config: &config,
            net_config,
            client: client.clone(),
            transaction_pool: transaction_pool.clone(),
            spawn_handle: task_manager.spawn_handle(),
            import_queue,
            block_announce_validator_builder: None,
            warp_sync_params,
            block_relay: None,
        })?;

    if config.offchain_worker.enabled {
        task_manager.spawn_handle().spawn(
            "offchain-workers-runner",
            "offchain-worker",
            sc_offchain::OffchainWorkers::new(sc_offchain::OffchainWorkerOptions {
                runtime_api_provider: client.clone(),
                is_validator: config.role.is_authority(),
                keystore: Some(keystore_container.keystore()),
                offchain_db: backend.offchain_storage(),
                transaction_pool: Some(OffchainTransactionPoolFactory::new(transaction_pool.clone())),
                network_provider: network.clone(),
                enable_http_requests: true,
                custom_extensions: |_| vec![],
            })
            .run(client.clone(), task_manager.spawn_handle())
            .boxed(),
        );
    }

    let role = config.role.clone();
    let force_authoring = config.force_authoring;
    let backoff_authoring_blocks: Option<()> = None;
    let name = config.network.node_name.clone();
    let enable_grandpa = !config.disable_grandpa && sealing.is_default();
    let prometheus_registry = config.prometheus_registry().cloned();
    let starting_block = client.info().best_number;

    // Channel for the rpc handler to communicate with the authorship task.
    let (command_sink, commands_stream) = match sealing {
        SealingMode::Manual => {
            let (sender, receiver) = mpsc::channel(1000);
            (Some(sender), Some(receiver))
        }
        _ => (None, None),
    };

    let overrides = overrides_handle(client.clone());
    let config_dir: PathBuf = config.data_path.clone();
    let genesis_data = OnDiskGenesisConfig(config_dir);
    let starknet_rpc_params = StarknetDeps {
        client: client.clone(),
        madara_backend: madara_backend.clone(),
        overrides: overrides.clone(),
        sync_service: sync_service.clone(),
        starting_block,
        genesis_provider: genesis_data.into(),
    };

    let rpc_extensions_builder = {
        let client = client.clone();
        let pool = transaction_pool.clone();
        let graph = transaction_pool.pool().clone();
        let command_sink = command_sink.clone();

        Box::new(move |deny_unsafe, _| {
            let deps = crate::rpc::FullDeps {
                client: client.clone(),
                pool: pool.clone(),
                graph: graph.clone(),
                deny_unsafe,
                starknet: starknet_rpc_params.clone(),
                command_sink: command_sink.clone(),
            };
            crate::rpc::create_full(deps).map_err(Into::into)
        })
    };

    let _rpc_handlers = sc_service::spawn_tasks(sc_service::SpawnTasksParams {
        network: network.clone(),
        client: client.clone(),
        keystore: keystore_container.keystore(),
        task_manager: &mut task_manager,
        transaction_pool: transaction_pool.clone(),
        rpc_builder: rpc_extensions_builder,
        backend: backend.clone(),
        system_rpc_tx,
        tx_handler_controller,
        sync_service: sync_service.clone(),
        config,
        telemetry: telemetry.as_mut(),
    })?;

    task_manager.spawn_essential_handle().spawn(
        "mc-mapping-sync-worker",
        Some(MADARA_TASK_GROUP),
        MappingSyncWorker::<_, _, StarknetHasher>::new(
            client.import_notification_stream(),
            Duration::new(6, 0),
            client.clone(),
            backend.clone(),
            3,
            0,
        )
        .for_each(|()| future::ready(())),
    );

    let (block_sender, block_receiver) = tokio::sync::mpsc::channel::<DeoxysBlock>(100);
    let (state_update_sender, state_update_receiver) = tokio::sync::mpsc::channel::<StateUpdateWrapper>(100);
    let (class_sender, class_receiver) = tokio::sync::mpsc::channel::<ClassUpdateWrapper>(100);

    let sender_config = mc_sync::SenderConfig {
        block_sender,
        state_update_sender,
        command_sink: command_sink.unwrap().clone(),
        class_sender,
        overrides,
    };

    task_manager.spawn_essential_handle().spawn(
        "starknet-sync-worker",
        Some("madara"),
        starknet_sync_worker::sync(fetch_config, sender_config, rpc_port, l1_url, Arc::clone(&client)),
    );

    // manual-seal authorship
    if !sealing.is_default() {
        run_manual_seal_authorship(
            block_receiver,
            state_update_receiver,
            class_receiver,
            sealing,
            client,
            transaction_pool,
            select_chain,
            block_import,
            &task_manager,
            prometheus_registry.as_ref(),
            commands_stream,
            telemetry,
        )?;

        network_starter.start_network();

        return Ok(task_manager);
    }

    if role.is_authority() {
        let proposer_factory = ProposerFactory::new(
            task_manager.spawn_handle(),
            client.clone(),
            transaction_pool.clone(),
            prometheus_registry.as_ref(),
            telemetry.as_ref().map(|x| x.handle()),
        );

        let slot_duration = sc_consensus_aura::slot_duration(&*client)?;

        let aura = sc_consensus_aura::start_aura::<AuraPair, _, _, _, _, _, _, _, _, _, _>(StartAuraParams {
            slot_duration,
            client: client.clone(),
            select_chain,
            block_import,
            proposer_factory,
            create_inherent_data_providers: move |_, ()| {
                let offchain_storage = backend.offchain_storage();
                async move {
                    let timestamp = sp_timestamp::InherentDataProvider::from_system_time();

                    let slot = sp_consensus_aura::inherents::InherentDataProvider::from_timestamp_and_slot_duration(
                        *timestamp,
                        slot_duration,
                    );

                    let ocw_storage = offchain_storage.clone();
                    let prefix = &STORAGE_PREFIX;
                    let key = SEQ_ADDR_STORAGE_KEY;

                    let sequencer_address = if let Some(storage) = ocw_storage {
                        SeqAddrInherentDataProvider::try_from(
                            storage.get(prefix, key).unwrap_or(DEFAULT_SEQUENCER_ADDRESS.to_vec()),
                        )
                        .unwrap_or_default()
                    } else {
                        SeqAddrInherentDataProvider::default()
                    };

                    Ok((slot, timestamp, sequencer_address))
                }
            },
            force_authoring,
            backoff_authoring_blocks,
            keystore: keystore_container.keystore(),
            sync_oracle: sync_service.clone(),
            justification_sync_link: sync_service.clone(),
            block_proposal_slot_portion: SlotProportion::new(2f32 / 3f32),
            max_block_proposal_slot_portion: None,
            telemetry: telemetry.as_ref().map(|x| x.handle()),
            compatibility_mode: Default::default(),
        })?;

        // the AURA authoring task is considered essential, i.e. if it
        // fails we take down the service with it.
        task_manager.spawn_essential_handle().spawn_blocking("aura", Some("block-authoring"), aura);
    }

    if enable_grandpa {
        // if the node isn't actively participating in consensus then it doesn't
        // need a keystore, regardless of which protocol we use below.
        let keystore = if role.is_authority() { Some(keystore_container.keystore()) } else { None };

        let grandpa_config = sc_consensus_grandpa::Config {
            // FIXME #1578 make this available through chainspec
            gossip_duration: Duration::from_millis(333),
            justification_generation_period: GRANDPA_JUSTIFICATION_PERIOD,
            name: Some(name),
            observer_enabled: false,
            keystore,
            local_role: role,
            telemetry: telemetry.as_ref().map(|x| x.handle()),
            protocol_name: grandpa_protocol_name,
        };

        // start the full GRANDPA voter
        // NOTE: non-authorities could run the GRANDPA observer protocol, but at
        // this point the full voter should provide better guarantees of block
        // and vote data availability than the observer. The observer has not
        // been tested extensively yet and having most nodes in a network run it
        // could lead to finality stalls.
        let grandpa_config = sc_consensus_grandpa::GrandpaParams {
            config: grandpa_config,
            link: grandpa_link,
            network,
            sync: Arc::new(sync_service),
            voting_rule: sc_consensus_grandpa::VotingRulesBuilder::default().build(),
            prometheus_registry,
            shared_voter_state: SharedVoterState::empty(),
            telemetry: telemetry.as_ref().map(|x| x.handle()),
            offchain_tx_pool_factory: OffchainTransactionPoolFactory::new(transaction_pool.clone()),
        };

        // the GRANDPA voter task is considered infallible, i.e.
        // if it fails we take down the service with it.
        task_manager.spawn_essential_handle().spawn_blocking(
            "grandpa-voter",
            None,
            sc_consensus_grandpa::run_grandpa_voter(grandpa_config)?,
        );
    }

    network_starter.start_network();

    Ok(task_manager)
}

#[allow(clippy::too_many_arguments)]
fn run_manual_seal_authorship(
    block_receiver: tokio::sync::mpsc::Receiver<DeoxysBlock>,
    state_update_receiver: tokio::sync::mpsc::Receiver<StateUpdateWrapper>,
    class_receiver: tokio::sync::mpsc::Receiver<ClassUpdateWrapper>,
    sealing: SealingMode,
    client: Arc<FullClient>,
    transaction_pool: Arc<FullPool<DBlockT, FullClient>>,
    select_chain: FullSelectChain,
    block_import: BoxBlockImport,
    task_manager: &TaskManager,
    prometheus_registry: Option<&Registry>,
    commands_stream: Option<mpsc::Receiver<sc_consensus_manual_seal::rpc::EngineCommand<Hash>>>,
    telemetry: Option<Telemetry>,
) -> Result<(), ServiceError>
where
    RuntimeApi: ConstructRuntimeApi<DBlockT, FullClient>,
    RuntimeApi: Send + Sync + 'static,
{
    let proposer_factory = ProposerFactory::new(
        task_manager.spawn_handle(),
        client.clone(),
        transaction_pool.clone(),
        prometheus_registry,
        telemetry.as_ref().map(|x| x.handle()),
    );

    thread_local!(static TIMESTAMP: RefCell<u64> = RefCell::new(0));

    /// Provide a mock duration starting at 0 in millisecond for timestamp inherent.
    /// Each call will increment timestamp by slot_duration making Aura think time has passed.
    struct MockTimestampInherentDataProvider;

    #[async_trait::async_trait]
    impl sp_inherents::InherentDataProvider for MockTimestampInherentDataProvider {
        async fn provide_inherent_data(
            &self,
            inherent_data: &mut sp_inherents::InherentData,
        ) -> Result<(), sp_inherents::Error> {
            TIMESTAMP.with(|x| {
                *x.borrow_mut() += deoxys_runtime::SLOT_DURATION;
                inherent_data.put_data(sp_timestamp::INHERENT_IDENTIFIER, &*x.borrow())
            })
        }

        async fn try_handle_error(
            &self,
            _identifier: &sp_inherents::InherentIdentifier,
            _error: &[u8],
        ) -> Option<Result<(), sp_inherents::Error>> {
            // The pallet never reports error.
            None
        }
    }

    let create_inherent_data_providers = move |_, ()| async move {
        let timestamp = MockTimestampInherentDataProvider;
        Ok(timestamp)
    };

    struct QueryBlockConsensusDataProvider<C> {
        _client: Arc<C>,

        /// The receiver that we're using to receive blocks.
        block_receiver: tokio::sync::Mutex<tokio::sync::mpsc::Receiver<DeoxysBlock>>,

        /// The receiver that we're using to receive state updates.
        state_update_receiver: tokio::sync::Mutex<tokio::sync::mpsc::Receiver<StateUpdateWrapper>>,

        /// The receiver that we're using to receive class updates.
        class_receiver: tokio::sync::Mutex<tokio::sync::mpsc::Receiver<ClassUpdateWrapper>>,
    }

    impl<B, C> ConsensusDataProvider<B> for QueryBlockConsensusDataProvider<C>
    where
        B: BlockT,
        C: ProvideRuntimeApi<B> + Send + Sync,
    {
        type Proof = ();

        fn create_digest(&self, _parent: &B::Header, _inherents: &InherentData) -> Result<Digest, Error> {
            // listening for new blocks
            let mut lock = self.block_receiver.try_lock().map_err(|e| Error::Other(e.into()))?;
            let block = lock.try_recv().map_err(|_| Error::EmptyTransactionPool)?;
            let block_digest_item: DigestItem =
                sp_runtime::DigestItem::PreRuntime(mp_digest_log::MADARA_ENGINE_ID, Encode::encode(&block));

            // listening for new state
            let mut lock = self.state_update_receiver.try_lock().map_err(|e| Error::Other(e.into()))?;
            let state_update = lock.try_recv().map_err(|_| Error::EmptyTransactionPool)?;
            let state_update_digest_item: DigestItem =
                sp_runtime::DigestItem::PreRuntime(mp_digest_log::STATE_ENGINE_ID, Encode::encode(&state_update));

            // listening for new classes
            let mut lock = self.class_receiver.try_lock().map_err(|e| Error::Other(e.into()))?;
            let class = lock.try_recv().map_err(|_| Error::EmptyTransactionPool)?;
            let class_digest_item: DigestItem =
                sp_runtime::DigestItem::PreRuntime(mp_digest_log::CLASS_ENGINE_ID, Encode::encode(&class));

            Ok(Digest { logs: vec![block_digest_item, state_update_digest_item, class_digest_item] })
        }

        fn append_block_import(
            &self,
            _parent: &B::Header,
            params: &mut BlockImportParams<B>,
            _inherents: &InherentData,
            _proof: Self::Proof,
        ) -> Result<(), Error> {
            params.post_digests.push(DigestItem::Other(vec![1]));
            Ok(())
        }
    }

    let manual_seal: BoxFuture<_> = match sealing {
        SealingMode::Manual => {
            Box::pin(sc_consensus_manual_seal::run_manual_seal(sc_consensus_manual_seal::ManualSealParams {
                block_import,
                env: proposer_factory,
                client: client.clone(),
                pool: transaction_pool,
                commands_stream: commands_stream.expect("Manual sealing requires a channel from RPC."),
                select_chain,
                consensus_data_provider: Some(Box::new(QueryBlockConsensusDataProvider {
                    _client: client,
                    block_receiver: tokio::sync::Mutex::new(block_receiver),
                    state_update_receiver: tokio::sync::Mutex::new(state_update_receiver),
                    class_receiver: tokio::sync::Mutex::new(class_receiver),
                })),
                create_inherent_data_providers,
            }))
        }
        SealingMode::Instant { finalize } => {
            let instant_seal_params = sc_consensus_manual_seal::InstantSealParams {
                block_import,
                env: proposer_factory,
                client,
                pool: transaction_pool,
                select_chain,
                consensus_data_provider: None,
                create_inherent_data_providers,
            };
            if finalize {
                Box::pin(sc_consensus_manual_seal::run_instant_seal_and_finalize(instant_seal_params))
            } else {
                Box::pin(sc_consensus_manual_seal::run_instant_seal(instant_seal_params))
            }
        }
        _ => unreachable!("Other sealing modes are not expected in manual-seal."),
    };

    // we spawn the future on a background thread managed by service.
    task_manager.spawn_essential_handle().spawn_blocking("manual-seal", None, manual_seal);
    Ok(())
}

type ChainOpsResult =
    Result<(Arc<FullClient>, Arc<FullBackend>, BasicQueue<DBlockT>, TaskManager, Arc<MadaraBackend>), ServiceError>;

pub fn new_chain_ops(config: &mut Configuration, cache_more_things: bool) -> ChainOpsResult {
    config.keystore = sc_service::config::KeystoreConfig::InMemory;
    let sc_service::PartialComponents { client, backend, import_queue, task_manager, other, .. } =
        new_partial::<_>(config, build_aura_grandpa_import_queue, cache_more_things, DeoxysBlock::default())?;
    Ok((client, backend, import_queue, task_manager, other.3))
}
