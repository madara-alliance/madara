//! Service and ServiceFactory implementation. Specialized wrapper over substrate service.

use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use deoxys_runtime::{self, RuntimeApi, SealingMode};
use futures::channel::mpsc;
use futures::future;
use futures::future::BoxFuture;
use futures::prelude::*;
use mc_db::DeoxysBackend;
use mc_genesis_data_provider::OnDiskGenesisConfig;
use mc_mapping_sync::MappingSyncWorker;
use mc_sync::fetch::fetchers::FetchConfig;
use mc_sync::metrics::block_metrics::BlockMetrics;
use mc_sync::starknet_sync_worker;
use mp_block::DeoxysBlock;
use mp_types::block::{DBlockT, DHashT, DHasherT};
use parity_scale_codec::Encode;
use reqwest::Url;
use sc_basic_authorship::ProposerFactory;
use sc_client_api::BlockchainEvents;
use sc_consensus::{BasicQueue, BlockImportParams};
use sc_consensus_manual_seal::{ConsensusDataProvider, Error};
pub use sc_executor::NativeElseWasmExecutor;
use sc_service::error::Error as ServiceError;
use sc_service::{new_db_backend, Configuration, TaskManager};
use sc_telemetry::{Telemetry, TelemetryWorker};
use sc_transaction_pool::FullPool;
use sp_api::{ConstructRuntimeApi, ProvideRuntimeApi};
use sp_inherents::InherentData;
use sp_runtime::testing::Digest;
use sp_runtime::traits::Block as BlockT;
use sp_runtime::DigestItem;

use crate::configs::db_config_dir;
use crate::genesis_block::DeoxysGenesisBlockBuilder;
use crate::rpc::StarknetDeps;
// Our native executor instance.
pub struct ExecutorDispatch;

const DEOXYS_TASK_GROUP: &str = "deoxys";

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

#[allow(clippy::type_complexity)]
pub fn new_partial<BIQ>(
    config: &Configuration,
    build_import_queue: BIQ,
    genesis_block: DeoxysBlock,
    backup_dir: Option<PathBuf>,
    restore_from_latest_backup: bool,
) -> Result<
    sc_service::PartialComponents<
        FullClient,
        FullBackend,
        FullSelectChain,
        sc_consensus::DefaultImportQueue<DBlockT>,
        sc_transaction_pool::FullPool<DBlockT, FullClient>,
        (BoxBlockImport, Option<Telemetry>, Arc<DeoxysBackend>),
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
    ) -> Result<(BasicImportQueue, BoxBlockImport), ServiceError>,
{
    let deoxys_backend = DeoxysBackend::open(
        // &config.database,
        &db_config_dir(config),
        backup_dir,
        restore_from_latest_backup,
        cache_more_things,
    )
    .unwrap();

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

    let genesis_block_builder = DeoxysGenesisBlockBuilder::<DBlockT, _, _>::new(
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
        DeoxysGenesisBlockBuilder<DBlockT, FullBackend, NativeElseWasmExecutor<ExecutorDispatch>>,
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

    let (import_queue, block_import) = build_import_queue(client.clone(), config, &task_manager)?;

    Ok(sc_service::PartialComponents {
        client,
        backend,
        task_manager,
        import_queue,
        keystore_container,
        select_chain,
        transaction_pool,
        other: (block_import, telemetry, Arc::clone(deoxys_backend)),
    })
}

/// Build the import queue for the template runtime (manual seal).
pub fn build_manual_seal_import_queue(
    client: Arc<FullClient>,
    config: &Configuration,
    task_manager: &TaskManager,
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
#[allow(clippy::too_many_arguments)] // grr
pub fn new_full(
    config: Configuration,
    sealing: SealingMode,
    l1_url: Url,
    fetch_config: FetchConfig,
    genesis_block: DeoxysBlock,
    starting_block: Option<u32>,
    backup_every_n_blocks: Option<usize>,
    backup_dir: Option<PathBuf>,
    restore_from_latest_backup: bool,
) -> Result<TaskManager, ServiceError> {
    let build_import_queue = build_manual_seal_import_queue;

    let sc_service::PartialComponents {
        client,
        backend,
        mut task_manager,
        import_queue,
        keystore_container,
        select_chain,
        transaction_pool,
        other: (block_import, mut telemetry, deoxys_backend),
    } = new_partial(&config, build_import_queue, genesis_block, backup_dir, restore_from_latest_backup)?;

    let net_config = sc_network::config::FullNetworkConfiguration::new(&config.network);

    let (network, system_rpc_tx, tx_handler_controller, network_starter, sync_service) =
        sc_service::build_network(sc_service::BuildNetworkParams {
            config: &config,
            net_config,
            client: client.clone(),
            transaction_pool: transaction_pool.clone(),
            spawn_handle: task_manager.spawn_handle(),
            import_queue,
            block_announce_validator_builder: None,
            warp_sync_params: None,
            block_relay: None,
        })?;

    let prometheus_registry = config.prometheus_registry().cloned();
    let block_metrics = prometheus_registry.and_then(|registry| BlockMetrics::register(&registry).ok());

    let best_block = DeoxysBackend::meta().current_sync_block().expect("getting current sync block") as _;
    let on_block =
        if starting_block.is_some() && starting_block >= Some(best_block) { starting_block } else { Some(best_block) };

    // Channel for the rpc handler to communicate with the authorship task.
    let (command_sink, commands_stream) = match sealing {
        SealingMode::Manual => {
            let (sender, receiver) = mpsc::channel(1000);
            (Some(sender), Some(receiver))
        }
        _ => (None, None),
    };

    let config_dir: PathBuf = config.data_path.clone();
    let genesis_data = OnDiskGenesisConfig(config_dir);
    let starknet_rpc_params = StarknetDeps {
        client: client.clone(),
        deoxys_backend: deoxys_backend.clone(),
        sync_service: sync_service.clone(),
        starting_block: on_block.unwrap(),
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
        Some(DEOXYS_TASK_GROUP),
        MappingSyncWorker::<_, _, DHasherT>::new(
            client.import_notification_stream(),
            Duration::new(6, 0),
            client.clone(),
            backend.clone(),
            3,
            0,
            block_metrics.clone(),
        )
        .for_each(|()| future::ready(())),
    );

    let (block_sender, block_receiver) = tokio::sync::mpsc::channel::<DeoxysBlock>(100);

    task_manager.spawn_essential_handle().spawn("starknet-sync-worker", Some(DEOXYS_TASK_GROUP), {
        let fut = starknet_sync_worker::sync(
            fetch_config,
            block_sender,
            command_sink.unwrap().clone(),
            l1_url,
            Arc::clone(&client),
            on_block.unwrap(),
            backup_every_n_blocks,
            block_metrics,
        );
        async { fut.await.unwrap() }
    });

    // manual-seal authorship
    if !sealing.is_default() {
        run_manual_seal_authorship(
            block_receiver,
            sealing,
            client,
            transaction_pool,
            select_chain,
            block_import,
            &task_manager,
            commands_stream,
            telemetry,
        )?;

        network_starter.start_network();

        return Ok(task_manager);
    }

    network_starter.start_network();

    Ok(task_manager)
}

#[allow(clippy::too_many_arguments)]
fn run_manual_seal_authorship(
    block_receiver: tokio::sync::mpsc::Receiver<DeoxysBlock>,
    sealing: SealingMode,
    client: Arc<FullClient>,
    transaction_pool: Arc<FullPool<DBlockT, FullClient>>,
    select_chain: FullSelectChain,
    block_import: BoxBlockImport,
    task_manager: &TaskManager,
    commands_stream: Option<mpsc::Receiver<sc_consensus_manual_seal::rpc::EngineCommand<DHashT>>>,
    _telemetry: Option<Telemetry>,
) -> Result<(), ServiceError>
where
    RuntimeApi: ConstructRuntimeApi<DBlockT, FullClient>,
    RuntimeApi: Send + Sync + 'static,
{
    let proposer_factory =
        ProposerFactory::new(task_manager.spawn_handle(), client.clone(), transaction_pool.clone(), None, None);

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
                sp_runtime::DigestItem::PreRuntime(mp_digest_log::DEOXYS_ENGINE_ID, Encode::encode(&block));

            Ok(Digest { logs: vec![block_digest_item] })
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
    Result<(Arc<FullClient>, Arc<FullBackend>, BasicQueue<DBlockT>, TaskManager, Arc<DeoxysBackend>), ServiceError>;

pub fn new_chain_ops(config: &mut Configuration) -> ChainOpsResult {
    config.keystore = sc_service::config::KeystoreConfig::InMemory;
    let sc_service::PartialComponents { client, backend, import_queue, task_manager, other, .. } =
        new_partial::<_>(config, build_manual_seal_import_queue, DeoxysBlock::default(), None, false)?;
    Ok((client, backend, import_queue, task_manager, other.2))
}
