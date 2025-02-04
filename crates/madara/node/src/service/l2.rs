use mc_db::MadaraBackend;
use mc_eth::state_update::L1HeadReceiver;
use mc_p2p::P2pCommands;
use mc_sync2::{
    import::{BlockImporter, BlockValidationConfig},
    SyncControllerConfig,
};
use mp_utils::service::{MadaraServiceId, PowerOfTwo, Service, ServiceId, ServiceRunner};
use std::sync::Arc;

use crate::cli::l2::L2SyncParams;

#[derive(Clone)]
struct StartArgs {
    p2p_commands: Option<P2pCommands>,
    l1_head_recv: L1HeadReceiver,
    db_backend: Arc<MadaraBackend>,
    params: L2SyncParams,
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
            stop_on_sync: this.params.stop_on_sync,
        };

        if let Some(starting_block) = this.params.unsafe_starting_block {
            // We state that starting_block - 1 is the chain head.
            this.db_backend.head_status().set_to_height(starting_block.checked_sub(1));
        }

        runner.service_loop(move |ctx| async move {
            if this.params.p2p_sync {
                use mc_sync2::p2p::{forward_sync, ForwardSyncConfig, P2pPipelineArguments};

                let Some(p2p_commands) = this.p2p_commands else {
                    anyhow::bail!("Cannot enable --p2p-sync without starting the peer-to-peer service using --p2p.")
                };
                let args = P2pPipelineArguments::new(this.db_backend, p2p_commands, importer);
                forward_sync(args, config, ForwardSyncConfig::default().disable_tries(this.params.disable_tries))
                    .run(ctx)
                    .await
            } else {
                let gateway = this.params.create_feeder_client(this.db_backend.chain_config().clone())?;
                mc_sync2::gateway::forward_sync(
                    this.db_backend,
                    importer,
                    gateway,
                    config,
                    mc_sync2::gateway::ForwardSyncConfig::default()
                        .disable_tries(this.params.disable_tries)
                        .no_sync_pending_block(this.params.no_pending_sync)
                        .keep_pre_v0_13_2_hashes(this.params.keep_pre_v0_13_2_hashes),
                )
                .run(ctx)
                .await
            }
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
