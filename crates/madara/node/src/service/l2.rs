use crate::cli::l2::L2SyncParams;
use mc_db::MadaraBackend;
use mc_eth::state_update::L1HeadReceiver;
use mc_gateway_client::GatewayProvider;
use mc_p2p::P2pCommands;
use mc_rpc::versions::admin::v0_1_0::MadaraStatusRpcApiV0_1_0Client;
use mc_sync::{
    import::{BlockImporter, BlockValidationConfig},
    SyncControllerConfig,
};
use mp_utils::service::{MadaraServiceId, Service, ServiceId, ServiceIdProvider, ServiceRunner};
use std::sync::Arc;
use url::Url;

#[derive(Clone, Debug)]
pub struct WarpUpdateConfig {
    /// The port used for nodes to make rpc calls during a warp update.
    pub warp_update_port_rpc: u16,
    /// The port used for nodes to send blocks during a warp update.
    pub warp_update_port_fgw: u16,
    /// Whether to shutdown the warp update sender once the migration has completed.
    pub warp_update_shutdown_sender: bool,
    /// Whether to shut down the warp update receiver once the migration has completed
    pub warp_update_shutdown_receiver: bool,
    /// A list of services to start once warp update has completed.
    pub deferred_service_start: Vec<MadaraServiceId>,
    /// A list of services to stop one warp update has completed.
    pub deferred_service_stop: Vec<MadaraServiceId>,
}

#[derive(Clone)]
struct StartArgs {
    p2p_commands: Option<P2pCommands>,
    l1_head_recv: L1HeadReceiver,
    db_backend: Arc<MadaraBackend>,
    params: L2SyncParams,
    warp_update: Option<WarpUpdateConfig>,
}

#[derive(Clone)]
pub struct SyncService {
    start_args: Option<StartArgs>,
    disabled: bool,
}

impl SyncService {
    pub async fn new(
        config: &L2SyncParams,
        db: &Arc<MadaraBackend>,
        mut p2p_commands: Option<P2pCommands>,
        l1_head_recv: L1HeadReceiver,
        warp_update: Option<WarpUpdateConfig>,
    ) -> anyhow::Result<Self> {
        if !config.p2p_sync {
            p2p_commands = None;
        }
        Ok(Self {
            start_args: (!config.l2_sync_disabled).then_some(StartArgs {
                p2p_commands,
                l1_head_recv,
                db_backend: db.clone(),
                params: config.clone(),
                warp_update,
            }),
            disabled: config.l2_sync_disabled,
        })
    }
}

#[async_trait::async_trait]
impl Service for SyncService {
    async fn start<'a>(&mut self, runner: ServiceRunner<'a>) -> anyhow::Result<()> {
        if self.disabled {
            return Ok(());
        }
        let this = self.start_args.take().expect("Service already started");
        let importer = Arc::new(BlockImporter::new(
            this.db_backend.clone(),
            BlockValidationConfig::default().trust_parent_hash(this.params.unsafe_starting_block.is_some()),
        ));

        let config = SyncControllerConfig {
            l1_head_recv: this.l1_head_recv,
            stop_at_block_n: this.params.sync_stop_at,
            global_stop_on_sync: this.params.stop_on_sync,
            stop_on_sync: false,
        };

        if let Some(starting_block) = this.params.unsafe_starting_block {
            // We state that starting_block - 1 is the chain head.
            this.db_backend.head_status().set_to_height(starting_block.checked_sub(1));
        }

        runner.service_loop(move |ctx| async move {
            // Warp update
            if let Some(WarpUpdateConfig {
                warp_update_port_rpc,
                warp_update_port_fgw,
                warp_update_shutdown_sender,
                warp_update_shutdown_receiver,
                deferred_service_start,
                deferred_service_stop,
            }) = this.warp_update
            {
                let client = jsonrpsee::http_client::HttpClientBuilder::default()
                    .build(format!("http://localhost:{warp_update_port_rpc}"))
                    .expect("Building client");

                if client.ping().await.is_err() {
                    tracing::error!(
                        "❗ Failed to connect to warp update sender on http://localhost:{warp_update_port_rpc}"
                    );
                    ctx.cancel_global();
                    return Ok(());
                }

                let gateway = Arc::new(GatewayProvider::new(
                    Url::parse(&format!("http://localhost:{warp_update_port_fgw}/gateway/"))
                        .expect("Failed to parse warp update sender gateway url. This should not fail in prod"),
                    Url::parse(&format!("http://localhost:{warp_update_port_fgw}/feeder_gateway/"))
                        .expect("Failed to parse warp update sender feeder gateway url. This should not fail in prod"),
                ));

                mc_sync::gateway::forward_sync(
                    this.db_backend.clone(),
                    importer.clone(),
                    gateway,
                    SyncControllerConfig {
                        l1_head_recv: config.l1_head_recv.clone(),
                        stop_at_block_n: None,
                        global_stop_on_sync: false,
                        stop_on_sync: true,
                    },
                    mc_sync::gateway::ForwardSyncConfig::default()
                        .disable_tries(this.params.disable_tries)
                        .no_sync_pending_block(true)
                        .keep_pre_v0_13_2_hashes(this.params.keep_pre_v0_13_2_hashes),
                )
                .run(ctx.clone())
                .await?;

                if warp_update_shutdown_sender {
                    if client.shutdown().await.is_err() {
                        tracing::error!("❗ Failed to shutdown warp update sender");
                        ctx.cancel_global();
                        return Ok(());
                    }

                    for svc_id in deferred_service_stop {
                        ctx.service_deactivate(svc_id).await;
                    }

                    for svc_id in deferred_service_start {
                        ctx.service_activate(svc_id).await;
                    }
                }

                if warp_update_shutdown_receiver {
                    return anyhow::Ok(());
                }
            }

            if this.params.p2p_sync {
                use mc_sync::p2p::{forward_sync, ForwardSyncConfig, P2pPipelineArguments};

                let ctx_ = ctx.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    ctx_.cancel_global();
                });

                let Some(p2p_commands) = this.p2p_commands else {
                    anyhow::bail!("Cannot enable --p2p-sync without starting the peer-to-peer service using --p2p.")
                };
                let args = P2pPipelineArguments::new(this.db_backend, p2p_commands, importer);
                forward_sync(args, config, ForwardSyncConfig::default().disable_tries(this.params.disable_tries))
                    .run(ctx)
                    .await
            } else {
                let gateway = this.params.create_feeder_client(this.db_backend.chain_config().clone())?;
                mc_sync::gateway::forward_sync(
                    this.db_backend,
                    importer,
                    gateway,
                    config,
                    mc_sync::gateway::ForwardSyncConfig::default()
                        .disable_tries(this.params.disable_tries)
                        .no_sync_pending_block(this.params.no_pending_sync)
                        .keep_pre_v0_13_2_hashes(this.params.keep_pre_v0_13_2_hashes),
                )
                .run(ctx)
                .await
            }
        })
    }
}

impl ServiceIdProvider for SyncService {
    #[inline(always)]
    fn id_provider(&self) -> impl ServiceId {
        MadaraServiceId::L2Sync
    }
}
