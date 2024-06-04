pub mod db;
pub mod prometheus;
pub mod rpc;
pub mod sync;
pub mod telemetry;

pub use db::*;
pub use prometheus::*;
pub use rpc::*;
pub use sync::*;
pub use telemetry::*;

#[derive(Clone, Debug, clap::Parser)]
pub struct RunCmd {
    /// The human-readable name for this node.
    /// It's used as network node name.
    #[arg(long, value_name = "NAME")]
    pub name: Option<String>,

    #[allow(missing_docs)]
    #[clap(flatten)]
    pub db_params: DbParams,

    #[allow(missing_docs)]
    #[clap(flatten)]
    pub sync_params: SyncParams,

    #[allow(missing_docs)]
    #[clap(flatten)]
    pub telemetry_params: TelemetryParams,

    #[allow(missing_docs)]
    #[clap(flatten)]
    pub prometheus_params: PrometheusParams,

    #[allow(missing_docs)]
    #[clap(flatten)]
    pub rpc_params: RpcParams,

    /// Run the TUI dashboard
    #[cfg(feature = "tui")]
    #[clap(long)]
    pub tui: bool,
}

impl RunCmd {
    pub async fn provide_node_name(&mut self) -> String {
        match &self.name {
            Some(name) => name.clone(),
            None => {
                let name = mc_sync::utility::get_random_pokemon_name()
                    .await
                    .unwrap_or_else(|e| {
                        log::warn!("Failed to get random pokemon name: {}", e);
                        "deoxys".to_string()
                    });

                self.name = Some(name.clone());
                name
            },
        }
    }
}
