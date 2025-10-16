use crate::cli::l2::L2SyncParams;
use mc_db::MadaraBackend;
use mc_gateway_client::GatewayProvider;
use mc_rpc::versions::admin::v0_1_0::MadaraStatusRpcApiV0_1_0Client;
use mc_settlement_client::state_update::L1HeadReceiver;
use mc_sync::{
    import::{BlockImporter, BlockValidationConfig},
    SyncControllerConfig,
};
use mp_utils::service::{MadaraServiceId, PowerOfTwo, Service, ServiceId, ServiceRunner};
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
    l1_head_recv: L1HeadReceiver,
    db_backend: Arc<MadaraBackend>,
    params: L2SyncParams,
    warp_update: Option<WarpUpdateConfig>,
    unsafe_starting_block_enabled: bool,
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
        l1_head_recv: L1HeadReceiver,
        warp_update: Option<WarpUpdateConfig>,
        unsafe_starting_block_enabled: bool,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            start_args: (!config.l2_sync_disabled).then_some(StartArgs {
                l1_head_recv,
                db_backend: db.clone(),
                params: config.clone(),
                warp_update,
                unsafe_starting_block_enabled,
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
            // TODO: all_verifications_disabled should be configured from the env
            BlockValidationConfig::default()
                .trust_parent_hash(this.unsafe_starting_block_enabled)
                .trust_state_root(this.unsafe_starting_block_enabled)
                .all_verifications_disabled(true),
        ));

        let config = SyncControllerConfig::default()
            .l1_head_recv(this.l1_head_recv)
            .stop_at_block_n(this.params.sync_stop_at)
            .global_stop_on_sync(this.params.stop_on_sync)
            .stop_on_sync(this.params.stop_on_sync)
            .no_pending_block(this.params.no_pending_sync);

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
                    SyncControllerConfig::default().stop_on_sync(true).no_pending_block(true),
                    mc_sync::gateway::ForwardSyncConfig::default()
                        .disable_tries(this.params.disable_tries)
                        .snap_sync(this.params.snap_sync)
                        .keep_pre_v0_13_2_hashes(this.params.keep_pre_v0_13_2_hashes())
                        .enable_bouncer_config_sync(this.params.bouncer_config_sync_enable),
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
                        ctx.service_remove(svc_id);
                    }

                    for svc_id in deferred_service_start {
                        ctx.service_add(svc_id);
                    }
                }

                if warp_update_shutdown_receiver {
                    return anyhow::Ok(());
                }
            }

            let gateway = this.params.create_feeder_client(this.db_backend.chain_config().clone())?;
            mc_sync::gateway::forward_sync(
                this.db_backend,
                importer,
                gateway,
                config,
                mc_sync::gateway::ForwardSyncConfig::default()
                    .disable_tries(this.params.disable_tries)
                    .snap_sync(this.params.snap_sync)
                    .keep_pre_v0_13_2_hashes(this.params.keep_pre_v0_13_2_hashes())
                    .enable_bouncer_config_sync(this.params.bouncer_config_sync_enable),
            )
            .run(ctx)
            .await
        });

        Ok(())
    }
}

impl ServiceId for SyncService {
    #[inline(always)]
    fn svc_id(&self) -> PowerOfTwo {
        MadaraServiceId::L2Sync.svc_id()
    }
}
