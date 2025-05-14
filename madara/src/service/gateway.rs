use crate::{
    cli::GatewayParams,
    submit_tx::{MakeSubmitTransactionSwitch, MakeSubmitValidatedTransactionSwitch},
};
use mc_db::MadaraBackend;
use mp_utils::service::{MadaraServiceId, PowerOfTwo, Service, ServiceId, ServiceRunner};
use std::sync::Arc;

#[derive(Clone)]
pub struct GatewayService {
    config: GatewayParams,
    db_backend: Arc<MadaraBackend>,
    submit_tx_provider: MakeSubmitTransactionSwitch,
    submit_validated_tx_provider: Option<MakeSubmitValidatedTransactionSwitch>,
}

impl GatewayService {
    pub async fn new(
        config: GatewayParams,
        db_backend: Arc<MadaraBackend>,
        submit_tx_provider: MakeSubmitTransactionSwitch,
        submit_validated_tx_provider: Option<MakeSubmitValidatedTransactionSwitch>,
    ) -> anyhow::Result<Self> {
        Ok(Self { config, db_backend, submit_tx_provider, submit_validated_tx_provider })
    }
}

#[async_trait::async_trait]
impl Service for GatewayService {
    async fn start<'a>(&mut self, runner: ServiceRunner<'a>) -> anyhow::Result<()> {
        let GatewayService { config, db_backend, submit_tx_provider, submit_validated_tx_provider } = self.clone();

        runner.service_loop(move |ctx| {
            let submit_tx = Arc::new(submit_tx_provider.make(ctx.clone()));
            let submit_validated_tx = submit_validated_tx_provider.map(|s| Arc::new(s.make(ctx.clone())) as _);

            mc_gateway_server::service::start_server(
                db_backend,
                submit_tx,
                submit_validated_tx,
                ctx,
                config.as_gateway_server_config(),
            )
        });
        Ok(())
    }
}

impl ServiceId for GatewayService {
    #[inline(always)]
    fn svc_id(&self) -> PowerOfTwo {
        MadaraServiceId::Gateway.svc_id()
    }
}
